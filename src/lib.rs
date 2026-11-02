//! openshell-annotate: policy-as-code for OpenShell via `@openshell` source directives.
pub mod ast;
pub mod config;
pub mod diff_logs;
pub mod directive;
pub mod docs;
pub mod drift;
pub mod generate;
pub mod lexer;
pub mod lint;
pub mod providers;
pub mod report;
pub mod scan;
pub mod spec;
pub mod validate;
