use std::convert::Infallible;

use gix::{Repository, refs::Category};
use miette::Diagnostic;

#[derive(Debug, Diagnostic, thiserror::Error)]
pub enum GitError {
    #[error("failed to open git repository")]
    Open(#[from] gix::open::Error),
    #[error("failed to resolve HEAD reference")]
    FindRef(#[from] gix::reference::find::existing::Error),
    #[error("failed to retrieve dirty status")]
    Dirty(#[from] gix::status::is_dirty::Error),
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

#[derive(Default)]
pub struct State {
    pub dirty: bool,
    pub tag: Option<String>,
    pub commit: String,
}

pub async fn state() -> Result<State, GitError> {
    let repo = gix::open(".")?;
    let mut head = repo.head()?;
    let mut state = State {
        dirty: is_dirty(&repo)?,
        ..State::default()
    };

    if let Some(ref_name) = head.referent_name() {
        if let Some((Category::Tag, name)) = ref_name.category_and_short_name() {
            state.tag = Some(name.to_string());
        }
    }

    if let Ok(commit) = head.peel_to_commit_in_place() {
        state.commit = commit.id.to_hex().to_string();
    }

    Ok(state)
}
