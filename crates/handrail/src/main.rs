//! Handrail: policy packs for coding agents — enforced where the agent allows it, honest
//! where it doesn't. This version targets Claude Code.

mod change;
mod context;
mod embedded;
mod show;

use clap::{Args, Parser, Subcommand};
use context::{Ctx, Overrides};
use handrail_claude::Intent;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "handrail",
    version,
    about = "Policy packs for coding agents: enforced where the agent allows it, honest where it doesn't.",
    after_help = "Unofficial project; not affiliated with Anthropic or any agent vendor. https://github.com/martinx/handrail"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
    /// Use this directory instead of the system managed policy directory (testing only)
    #[arg(long, global = true, hide = true)]
    managed_root: Option<PathBuf>,
    /// Use this directory instead of ~/.claude (testing only)
    #[arg(long, global = true, hide = true)]
    user_dir: Option<PathBuf>,
    /// Pretend this Claude Code version is installed (testing only)
    #[arg(long, global = true, hide = true)]
    claude_version: Option<String>,
}

#[derive(Args)]
struct ChangeOpts {
    /// Show the plan without writing anything
    #[arg(long)]
    dry_run: bool,
    /// Do not ask for confirmation
    #[arg(long, short = 'y')]
    yes: bool,
}

impl From<&ChangeOpts> for change::Opts {
    fn from(o: &ChangeOpts) -> Self {
        change::Opts {
            dry_run: o.dry_run,
            yes: o.yes,
        }
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// List packs by category
    List {
        /// Only this category
        #[arg(long)]
        category: Option<String>,
    },
    /// Show everything about one pack: what it protects, its tradeoffs and limits
    Show { pack: String },
    /// List profiles (named sets of packs)
    Profiles,
    /// What is installed, and anything that would make it ineffective
    Status,
    /// Enable one or more packs
    Enable {
        #[arg(required = true)]
        packs: Vec<String>,
        #[command(flatten)]
        opts: ChangeOpts,
    },
    /// Disable packs, or everything with --all
    Disable {
        packs: Vec<String>,
        /// Remove every pack and local rule; leaves nothing behind
        #[arg(long, conflicts_with = "packs")]
        all: bool,
        #[command(flatten)]
        opts: ChangeOpts,
    },
    /// Make the installed packs exactly this profile
    Use {
        profile: String,
        #[command(flatten)]
        opts: ChangeOpts,
    },
    /// Your own rules, added to the enforced instructions
    Rule {
        #[command(subcommand)]
        cmd: RuleCmd,
    },
    /// Undo the last change
    Rollback {
        #[command(flatten)]
        opts: ChangeOpts,
    },
    /// Check that the installed policy is actually in force
    Doctor,
    /// The built-in catalog as JSON (used to build the website)
    #[command(name = "__catalog-json", hide = true)]
    CatalogJson,
    /// Privileged step, run through sudo by the commands above
    #[command(name = "__apply", hide = true)]
    PrivilegedApply {
        #[arg(long)]
        intent: PathBuf,
        #[arg(long)]
        expect: String,
    },
}

#[derive(Subcommand)]
enum RuleCmd {
    /// Add a rule
    Add {
        text: String,
        #[command(flatten)]
        opts: ChangeOpts,
    },
    /// List your rules
    List,
    /// Remove a rule by its number in `handrail rule list`
    Remove {
        number: usize,
        #[command(flatten)]
        opts: ChangeOpts,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let ctx = match Ctx::new(Overrides {
        managed_root: cli.managed_root,
        user_dir: cli.user_dir,
        claude_version: cli.claude_version,
    }) {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let result = match cli.cmd {
        Cmd::List { category } => {
            show::list(&ctx, category.as_deref());
            Ok(())
        }
        Cmd::Show { pack } => show::show(&ctx, &pack),
        Cmd::Profiles => {
            show::profiles(&ctx);
            Ok(())
        }
        Cmd::Status => {
            show::status(&ctx);
            Ok(())
        }
        Cmd::Doctor => {
            show::doctor(&ctx);
            Ok(())
        }
        Cmd::Enable { packs, opts } => {
            let mut i = ctx.current_intent();
            let unknown: Vec<&String> = packs
                .iter()
                .filter(|p| !ctx.catalog.packs.contains_key(*p))
                .collect();
            if let Some(u) = unknown.first() {
                return fail(format!("no pack named \"{u}\" (see: handrail list)"));
            }
            i.packs.extend(packs);
            change::run(&ctx, i, &(&opts).into())
        }
        Cmd::Disable { packs, all, opts } => {
            let mut i = ctx.current_intent();
            if all {
                i.packs.clear();
                i.local_rules.clear();
            } else if packs.is_empty() {
                return fail("disable what? Name packs, or use --all".into());
            } else {
                for p in &packs {
                    if !i.packs.remove(p) {
                        eprintln!("! {p} is not installed; skipping");
                    }
                }
            }
            change::run(&ctx, i, &(&opts).into())
        }
        Cmd::Use { profile, opts } => {
            let Some(pr) = ctx.catalog.profiles.get(&profile) else {
                let names: Vec<&String> = ctx.catalog.profiles.keys().collect();
                return fail(format!(
                    "no profile named \"{profile}\" (available: {names:?})"
                ));
            };
            let cur = ctx.current_intent();
            let i = Intent {
                packs: pr.packs.iter().cloned().collect(),
                local_rules: cur.local_rules,
                claude_version: cur.claude_version,
            };
            change::run(&ctx, i, &(&opts).into())
        }
        Cmd::Rule { cmd } => match cmd {
            RuleCmd::List => {
                let rules = ctx.current_intent().local_rules;
                if rules.is_empty() {
                    println!("No local rules. Add one: handrail rule add \"...\"");
                }
                rules
                    .iter()
                    .enumerate()
                    .for_each(|(n, r)| println!("{}. {r}", n + 1));
                Ok(())
            }
            RuleCmd::Add { text, opts } => {
                let mut i = ctx.current_intent();
                if text.trim().is_empty() {
                    return fail("the rule is empty".into());
                }
                i.local_rules.push(text.trim().to_string());
                change::run(&ctx, i, &(&opts).into())
            }
            RuleCmd::Remove { number, opts } => {
                let mut i = ctx.current_intent();
                if number == 0 || number > i.local_rules.len() {
                    return fail(format!(
                        "there is no rule {number} (see: handrail rule list)"
                    ));
                }
                i.local_rules.remove(number - 1);
                change::run(&ctx, i, &(&opts).into())
            }
        },
        Cmd::Rollback { opts } => change::rollback(&ctx, &(&opts).into()),
        Cmd::CatalogJson => {
            println!("{}", show::catalog_json(&ctx));
            Ok(())
        }
        Cmd::PrivilegedApply { intent, expect } => change::privileged_apply(&ctx, &intent, &expect),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(e),
    }
}

fn fail(msg: String) -> ExitCode {
    eprintln!("error: {msg}");
    ExitCode::from(1)
}
