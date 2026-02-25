use thiserror::Error;

#[derive(Debug, Error)]
pub enum NameError {
    #[error("worktree name must be between 1 and 48 characters")]
    InvalidLength,
    #[error("worktree name must start with a lowercase letter or digit")]
    InvalidFirstCharacter,
    #[error("worktree name contains invalid character '{character}'")]
    InvalidCharacter { character: char },
}

pub fn validate_worktree_name(name: &str) -> Result<(), NameError> {
    if name.is_empty() || name.len() > 48 {
        return Err(NameError::InvalidLength);
    }

    let mut characters = name.chars();
    let first = characters.next().expect("validated non-empty name");

    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(NameError::InvalidFirstCharacter);
    }

    for character in characters {
        if character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || character == '_'
            || character == '-'
        {
            continue;
        }

        return Err(NameError::InvalidCharacter { character });
    }

    Ok(())
}

pub fn sanitize_repo_component(value: &str) -> String {
    let mut output = String::with_capacity(value.len());

    for character in value.chars() {
        if character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || character == '_'
            || character == '-'
        {
            output.push(character);
        } else if character.is_ascii_uppercase() {
            output.push(character.to_ascii_lowercase());
        } else {
            output.push('-');
        }
    }

    if output.is_empty() {
        return "repo".to_string();
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_worktree_name_accepts_valid_input() {
        assert!(validate_worktree_name("feature_1").is_ok());
        assert!(validate_worktree_name("a").is_ok());
        assert!(validate_worktree_name("a-b").is_ok());
    }

    #[test]
    fn validate_worktree_name_rejects_invalid_input() {
        assert!(matches!(
            validate_worktree_name("Feature"),
            Err(NameError::InvalidFirstCharacter)
        ));
        assert!(matches!(
            validate_worktree_name("feature/one"),
            Err(NameError::InvalidCharacter { .. })
        ));
    }

    #[test]
    fn sanitize_repo_component_normalizes_characters() {
        assert_eq!(sanitize_repo_component("Project Repo"), "project-repo");
        assert_eq!(sanitize_repo_component("repo_name"), "repo_name");
    }
}
