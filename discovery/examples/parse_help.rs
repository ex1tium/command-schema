//! Basic help text parsing example.
//!
//! Demonstrates how to use `parse_help_text()` to extract a structured schema
//! from pre-captured help output without executing any commands.
//!
//! # Usage
//!
//! ```bash
//! cargo run -p command-schema-discovery --example parse_help
//! ```

use command_schema_discovery::parse_help_text;

fn main() {
    // Example help text (Clap-style)
    let help_text = r#"
Usage: mycli [OPTIONS] <COMMAND>

A fictional CLI tool for demonstration

Commands:
  init     Initialize a new project
  build    Build the project
  deploy   Deploy to production
  help     Print this message or the help of the given subcommand(s)

Options:
  -v, --verbose          Enable verbose output
  -q, --quiet            Suppress all output
  -c, --config <FILE>    Path to config file [default: config.toml]
      --no-color         Disable colored output
  -j, --jobs <N>         Number of parallel jobs [default: 4]
  -h, --help             Print help
  -V, --version          Print version
"#;

    // Parse the help text
    let result = parse_help_text("mycli", help_text);

    // Check if parsing succeeded
    println!("Parsing succeeded: {}", result.success);
    println!("Detected format: {:?}", result.detected_format);

    if !result.warnings.is_empty() {
        println!("\nWarnings:");
        for warning in &result.warnings {
            println!("  - {warning}");
        }
    }

    // Inspect the extracted schema
    if let Some(schema) = &result.schema {
        println!("\nCommand: {}", schema.command);
        println!("Confidence: {:.2}", schema.confidence);

        println!("\nGlobal flags ({}):", schema.global_flags.len());
        for flag in &schema.global_flags {
            let name = flag.canonical_name();
            let desc = flag.description.as_deref().unwrap_or("(no description)");
            let vtype = if flag.takes_value {
                format!(" <{:?}>", flag.value_type)
            } else {
                String::new()
            };
            println!("  {name}{vtype}  —  {desc}");
        }

        println!("\nSubcommands ({}):", schema.subcommands.len());
        for sub in &schema.subcommands {
            let desc = sub.description.as_deref().unwrap_or("(no description)");
            println!("  {}  —  {desc}", sub.name);
        }

        println!("\nPositional args ({}):", schema.positional.len());
        for arg in &schema.positional {
            let req = if arg.required { "required" } else { "optional" };
            println!("  {} ({req}, {:?})", arg.name, arg.value_type);
        }
    } else {
        println!("\nNo schema could be extracted.");
    }
}
