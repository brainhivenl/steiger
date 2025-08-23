use std::convert::Infallible;

use gix::{Head, Repository, refs::Category};
use miette::Diagnostic;

#[derive(Debug, Diagnostic, thiserror::Error)]
pub enum TagError {
    #[error("failed to open git repository")]
    Open(#[from] gix::open::Error),
    #[error("failed to resolve HEAD reference")]
    FindRef(#[from] gix::reference::find::existing::Error),
    #[error("failed to retrieve dirty status")]
    Dirty(#[from] gix::status::is_dirty::Error),
    #[error("unable to find tag")]
    NotFound,
}

fn parse_name(head: &mut Head<'_>) -> Result<String, TagError> {
    if let Some(ref_name) = head.referent_name() {
        if let Some((Category::Tag, name)) = ref_name.category_and_short_name() {
            return Ok(name.to_string());
        }
    }

    if let Ok(commit) = head.peel_to_commit_in_place() {
        return Ok(commit.id.to_hex_with_len(6).to_string());
    }

    Err(TagError::NotFound)
}

// Copied from gix but takes untracked files into account
fn is_dirty(repo: &Repository) -> Result<bool, gix::status::is_dirty::Error> {
    {
        let head_tree_id = repo.head_tree_id()?;
        let mut index_is_dirty = false;

        // Run this first as there is a high likelihood to find something, and it's very fast.
        repo.tree_index_status(
            &head_tree_id,
            &*repo.index_or_empty()?,
            None,
            gix::status::tree_index::TrackRenames::Disabled,
            |_, _, _| {
                index_is_dirty = true;
                Ok::<_, Infallible>(gix::diff::index::Action::Cancel)
            },
        )?;
        if index_is_dirty {
            return Ok(true);
        }
    }

    Ok(repo
        .status(gix::progress::Discard)?
        .untracked_files(gix::status::UntrackedFiles::Files)
        .index_worktree_rewrites(None)
        .index_worktree_submodules(gix::status::Submodule::AsConfigured { check_dirty: true })
        .into_index_worktree_iter(vec![])?
        .take_while(Result::is_ok)
        .next()
        .is_some())
}

pub async fn resolve() -> Result<String, TagError> {
    let repo = gix::open(".")?;
    let mut head = repo.head()?;
    let name = parse_name(&mut head)?;

    if is_dirty(&repo)? {
        return Ok(format!("{name}-dirty"));
    }

    Ok(name)
}
