use gix::{Head, refs::Category};

#[derive(Debug, thiserror::Error)]
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

pub async fn resolve() -> Result<String, TagError> {
    let repo = gix::open(".")?;
    let mut head = repo.head()?;
    let name = parse_name(&mut head)?;

    if repo.is_dirty()? {
        return Ok(format!("{name}~dirty"));
    }

    Ok(name)
}
