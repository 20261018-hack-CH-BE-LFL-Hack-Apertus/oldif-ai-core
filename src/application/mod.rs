pub mod assertions;
pub mod context;
pub mod interpretation;
pub mod page_locator;
pub mod passages;
pub mod prompts;
pub mod stages;
pub mod tools;
pub mod validation;
pub mod workflow;

#[cfg(test)]
mod assertion_tests;
#[cfg(test)]
mod budget_tests;
#[cfg(test)]
mod compact_prompt_test_support;
#[cfg(test)]
mod compact_prompt_tests;
#[cfg(test)]
mod context_projection_tests;
#[cfg(test)]
mod optional_proposals_tests;
