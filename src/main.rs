use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use offload::app::{App, Paths};
use offload::config::{self, Config};
use offload::{headless, tui};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "offload", version, about = "Fast, verified camera-card offloading with a beautiful terminal UI")]
struct Cli {
    /// Use this config file instead of ~/.config/offload/config.toml
    #[arg(long, global = true, value_name = "FILE")]
    config: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Copy a profile without the UI (for scripts)
    Run {
        /// Profile name(s); partial names work (e.g. `dji`). Several names copy them together as one batch.
        #[arg(value_name = "PROFILE")]
        profiles: Vec<String>,
        /// Copy every profile as one batch
        #[arg(short, long)]
        all: bool,
        /// Don't ask for confirmation
        #[arg(short, long)]
        yes: bool,
        /// Show what would be copied, write nothing
        #[arg(long)]
        dry_run: bool,
        /// Move: delete each original from the card right after its copy is verified
        #[arg(long = "move", conflicts_with = "keep")]
        move_: bool,
        /// Copy: keep the originals (overrides `mode = "move"` in the config)
        #[arg(long = "copy", id = "keep")]
        keep: bool,
        /// One folder per day the footage was shot (instead of one for today)
        #[arg(long, conflicts_with = "by_run_date")]
        by_shoot_date: bool,
        /// One folder for today's offload (overrides `organize = "shoot"` in the config)
        #[arg(long)]
        by_run_date: bool,
        /// Also write and verify every file on this second drive/folder
        #[arg(long, value_name = "FOLDER", conflicts_with = "no_mirror")]
        mirror: Option<String>,
        /// Skip the configured mirror for this run
        #[arg(long)]
        no_mirror: bool,
        /// Don't hold the computer awake during the copy
        #[arg(long)]
        no_awake: bool,
        /// After a clean run: nothing, sleep or shutdown
        #[arg(long, value_name = "ACTION", value_parser = ["nothing", "sleep", "shutdown"])]
        when_done: Option<String>,
    },
    /// List profiles and whether their cards are connected
    Profiles,
    /// Write the built-in 'hectorir' config to disk so you can edit it
    Init {
        #[arg(long)]
        force: bool,
    },
    /// Print the config file location
    ConfigPath,
    /// Share camera setups as single-file profile packs
    Pack {
        #[command(subcommand)]
        cmd: PackCmd,
    },
}

#[derive(Subcommand)]
enum PackCmd {
    /// Write a profile to a pack file (or stdout)
    Export {
        profile: String,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Add the profile in a pack file to your config
    Import { file: PathBuf },
}

fn main() {
    if let Err(e) = real_main() {
        eprintln!("error: {e:#}");
        std::process::exit(2);
    }
}

fn real_main() -> Result<()> {
    let cli = Cli::parse();
    let cfg_path = cli.config.clone().unwrap_or_else(config::default_config_path);
    let paths = Paths { config: cfg_path.clone(), ..Paths::standard() };
    match cli.cmd {
        None => {
            let cfg = Config::load_or_default(&cfg_path)?;
            tui::run(App::new(cfg, paths))
        }
        Some(Cmd::Run { profiles, all, yes, dry_run, move_, keep, by_shoot_date, by_run_date, mirror, no_mirror, no_awake, when_done }) => {
            let mode = if move_ {
                Some(offload::engine::Mode::Move)
            } else if keep {
                Some(offload::engine::Mode::Copy)
            } else {
                None
            };
            let over = headless::Overrides {
                shoot_dates: if by_shoot_date {
                    Some(true)
                } else if by_run_date {
                    Some(false)
                } else {
                    None
                },
                mirror: if no_mirror { Some(String::new()) } else { mirror },
                keep_awake: no_awake.then_some(false),
                when_done,
            };
            let cfg = Config::load_or_default(&cfg_path)?;
            let stdin = std::io::stdin();
            let code = headless::run(
                &cfg,
                &headless::Options { names: &profiles, all, yes, dry_run, mode, over },
                &paths.history,
                &mut stdin.lock(),
                &mut std::io::stdout(),
            )?;
            std::process::exit(code);
        }
        Some(Cmd::Profiles) => {
            let cfg = Config::load_or_default(&cfg_path)?;
            for p in &cfg.profiles {
                let present = p.source_paths().iter().filter(|s| s.is_dir()).count();
                println!("{:<24} {}/{} sources connected  [{}]  -> {}", p.name, present, p.sources.len(), p.include.join(","), p.subfolder);
            }
            Ok(())
        }
        Some(Cmd::Init { force }) => {
            if cfg_path.exists() && !force {
                bail!("{} already exists (use --force to overwrite; a .bak is kept)", cfg_path.display());
            }
            Config::default().save(&cfg_path)?;
            println!("Wrote {}", cfg_path.display());
            Ok(())
        }
        Some(Cmd::ConfigPath) => {
            println!("{}", cfg_path.display());
            Ok(())
        }
        Some(Cmd::Pack { cmd }) => match cmd {
            PackCmd::Export { profile, output } => {
                let cfg = Config::load_or_default(&cfg_path)?;
                let p = cfg.find_profile(&profile).with_context(|| format!("no profile matching '{profile}'"))?;
                let text = config::export_pack(p)?;
                match output {
                    Some(f) => {
                        std::fs::write(&f, text)?;
                        println!("Wrote {}", f.display());
                    }
                    None => print!("{text}"),
                }
                Ok(())
            }
            PackCmd::Import { file } => {
                let meta = std::fs::metadata(&file).with_context(|| format!("reading {}", file.display()))?;
                if meta.len() > config::MAX_CONFIG_BYTES {
                    bail!("pack file too large");
                }
                let profile = config::parse_pack(&std::fs::read_to_string(&file)?)?;
                let mut cfg = Config::load_or_default(&cfg_path)?;
                if cfg.profiles.iter().any(|p| p.name.eq_ignore_ascii_case(&profile.name)) {
                    bail!("you already have a profile named '{}'", profile.name);
                }
                println!("Imported '{}': sources {:?}, copies {:?}", profile.name, profile.sources, profile.include);
                cfg.profiles.push(profile);
                cfg.save(&cfg_path)?;
                println!("Saved to {}", cfg_path.display());
                Ok(())
            }
        },
    }
}
