use std::collections::VecDeque;

use anyhow::{Result, anyhow};
use inquire::{Confirm, Select, Text};

pub trait PromptDriver {
    fn confirm(&mut self, message: &str, default: bool) -> Result<bool>;
    fn input(&mut self, message: &str) -> Result<String>;
    fn select(&mut self, message: &str, options: &[String]) -> Result<usize>;
}

#[derive(Debug, Default)]
pub struct InquirePromptDriver;

impl InquirePromptDriver {
    pub fn new() -> Self {
        Self
    }
}

impl PromptDriver for InquirePromptDriver {
    fn confirm(&mut self, message: &str, default: bool) -> Result<bool> {
        Ok(Confirm::new(message).with_default(default).prompt()?)
    }

    fn input(&mut self, message: &str) -> Result<String> {
        Ok(Text::new(message).prompt()?)
    }

    fn select(&mut self, message: &str, options: &[String]) -> Result<usize> {
        let selected = Select::new(message, options.to_vec()).prompt()?;

        options
            .iter()
            .position(|option| option == &selected)
            .ok_or_else(|| anyhow!("selected option was not found in options list"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptedPromptResponse {
    Confirm(bool),
    Input(String),
    Select(usize),
}

#[derive(Debug, Default)]
pub struct ScriptedPromptDriver {
    responses: VecDeque<ScriptedPromptResponse>,
}

impl ScriptedPromptDriver {
    pub fn new(responses: Vec<ScriptedPromptResponse>) -> Self {
        Self {
            responses: responses.into(),
        }
    }

    fn next_response(&mut self) -> Result<ScriptedPromptResponse> {
        self.responses
            .pop_front()
            .ok_or_else(|| anyhow!("prompt response queue is empty"))
    }
}

impl PromptDriver for ScriptedPromptDriver {
    fn confirm(&mut self, _message: &str, _default: bool) -> Result<bool> {
        match self.next_response()? {
            ScriptedPromptResponse::Confirm(value) => Ok(value),
            unexpected => Err(anyhow!("expected confirm response, got {unexpected:?}")),
        }
    }

    fn input(&mut self, _message: &str) -> Result<String> {
        match self.next_response()? {
            ScriptedPromptResponse::Input(value) => Ok(value),
            unexpected => Err(anyhow!("expected input response, got {unexpected:?}")),
        }
    }

    fn select(&mut self, _message: &str, _options: &[String]) -> Result<usize> {
        match self.next_response()? {
            ScriptedPromptResponse::Select(value) => Ok(value),
            unexpected => Err(anyhow!("expected select response, got {unexpected:?}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripted_prompt_returns_values_in_order() {
        let mut prompt = ScriptedPromptDriver::new(vec![
            ScriptedPromptResponse::Confirm(true),
            ScriptedPromptResponse::Input("feature-a".to_string()),
            ScriptedPromptResponse::Select(2),
        ]);

        let options = vec!["a".to_string(), "b".to_string(), "c".to_string()];

        assert!(prompt.confirm("confirm", false).expect("confirm"));
        assert_eq!(prompt.input("input").expect("input"), "feature-a");
        assert_eq!(prompt.select("select", &options).expect("select"), 2);
    }

    #[test]
    fn scripted_prompt_errors_on_type_mismatch() {
        let mut prompt =
            ScriptedPromptDriver::new(vec![ScriptedPromptResponse::Input("x".to_string())]);
        let error = prompt.confirm("confirm", false).expect_err("should fail");
        assert!(error.to_string().contains("expected confirm response"));
    }

    #[test]
    fn scripted_prompt_errors_when_exhausted() {
        let mut prompt = ScriptedPromptDriver::new(vec![]);
        let error = prompt.input("input").expect_err("should fail");
        assert!(error.to_string().contains("queue is empty"));
    }
}
