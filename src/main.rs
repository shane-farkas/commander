//! `commander` — a Midnight Commander-style dual-pane file UI for agentic CLIs.
//!
//! Two modes, one binary:
//!   commander tui [DIR]   Launch the interactive dual-pane file manager.
//!   commander mcp         Run the stdio MCP server (registered by the host).

use std::process::ExitCode;

use commander_mcp::serve;
use commander_tui::{run, Outcome};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_default();

    let result = match cmd.as_str() {
        "tui" => run_tui(args.next()),
        "mcp" => serve().map_err(Into::into),
        "help" | "--help" | "-h" | "" => {
            print_usage();
            return ExitCode::SUCCESS;
        }
        other => {
            eprintln!("commander: unknown command '{other}'\n");
            print_usage();
            return ExitCode::from(2);
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("commander: error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run_tui(dir_arg: Option<String>) -> anyhow::Result<()> {
    let start = match dir_arg {
        Some(d) => std::path::PathBuf::from(d),
        None => std::env::current_dir()?,
    };
    match run(start)? {
        Outcome::Sent { count, action } => {
            let act = action.map(|a| format!(" (action: {a})")).unwrap_or_default();
            println!("Sent {count} item(s) to the agent{act}.");
        }
        Outcome::Cancelled => println!("Cancelled. Nothing sent."),
    }
    Ok(())
}

fn print_usage() {
    println!(
        "commander — dual-pane file UI for agentic CLIs\n\n\
         USAGE:\n  \
           commander tui [DIR]   Launch the interactive dual-pane file manager\n  \
           commander mcp         Run the stdio MCP server (for the agent host)\n  \
           commander help        Show this help\n"
    );
}
