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
        /// Profile name (partial names work, e.g. "dji")
        profile: String,
        /// Don't ask for confirmation
        #[arg(short, long)]
        yes: bool,
        /// Show what would be copied, write nothing
        #[arg(long)]
        dry_run: bool,
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
        Some(Cmd::Run { profile, yes, dry_run }) => {
            let cfg = Config::load_or_default(&cfg_path)?;
            let stdin = std::io::stdin();
            let code = headless::run(&cfg, &profile, yes, dry_run, &paths.history, &mut stdin.lock(), &mut std::io::stdout())?;
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
