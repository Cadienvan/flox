use std::{path::Path, str::FromStr};

use anyhow::{Context, Result};
use async_trait::async_trait;
use crossterm::tty::IsTty;
use flox_rust_sdk::{
    flox::{Flox, ResolvedInstallableMatch},
    prelude::{Channel, ChannelRegistry},
};
use indoc::indoc;
use itertools::Itertools;
use log::{debug, warn};
use once_cell::sync::Lazy;
use tempfile::TempDir;

use std::collections::HashSet;

use flox_rust_sdk::flox::FloxInstallable;
use flox_rust_sdk::prelude::Installable;

pub mod colors;
pub mod dialog;
pub mod init;
use std::borrow::Cow;

use regex::Regex;

use crate::config::Config;
use crate::utils::dialog::InquireExt;

static NIX_IDENTIFIER_SAFE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"^[a-zA-Z0-9_-]+$"#).unwrap());

struct Flake {}

impl Flake {
    fn determine_default_flake(path_str: String) {
        let _path = Path::new(&path_str);
    }
}

pub fn init_channels() -> Result<ChannelRegistry> {
    let mut channels = ChannelRegistry::default();
    channels.register_channel("flox", Channel::from_str("github:flox/floxpkgs")?);
    channels.register_channel("nixpkgs", Channel::from_str("github:flox/nixpkgs/stable")?);
    channels.register_channel(
        "nixpkgs-flox",
        Channel::from_str("github:flox/nixpkgs-flox/master")?,
    );

    // generate these dynamically based on <?>
    channels.register_channel(
        "nixpkgs-stable",
        Channel::from_str("github:flox/nixpkgs/stable")?,
    );
    channels.register_channel(
        "nixpkgs-staging",
        Channel::from_str("github:flox/nixpkgs/staging")?,
    );
    channels.register_channel(
        "nixpkgs-unstable",
        Channel::from_str("github:flox/nixpkgs/unstable")?,
    );

    Ok(channels)
}

fn nix_str_safe<'a>(s: &'a str) -> Cow<'a, str> {
    if NIX_IDENTIFIER_SAFE.is_match(s) {
        s.into()
    } else {
        format!("{:?}", s).into()
    }
}

#[async_trait]
pub trait InstallableDef: FromStr + Default + Clone {
    const DEFAULT_PREFIXES: &'static [(&'static str, bool)];
    const DEFAULT_FLAKEREFS: &'static [&'static str];
    const INSTALLABLE: fn(&Self) -> String;
    const SUBCOMMAND: &'static str;
    const DERIVATION_TYPE: &'static str;

    async fn resolve_matches(&self, flox: &Flox) -> Result<Vec<ResolvedInstallableMatch>> {
        Ok(flox
            .resolve_matches(
                &[Self::INSTALLABLE(self).parse()?],
                Self::DEFAULT_FLAKEREFS,
                Self::DEFAULT_PREFIXES,
                false,
            )
            .await?)
    }

    async fn resolve_installable(&self, flox: &Flox) -> Result<Installable> {
        Ok(resolve_installable_from_matches(
            Self::SUBCOMMAND,
            Self::DERIVATION_TYPE,
            self.resolve_matches(flox).await?,
        )
        .await?)
    }

    fn complete_inst(&self) -> Vec<(String, Option<String>)> {
        let inst = Self::INSTALLABLE(self);

        let config = Config::parse()
            .map_err(|e| debug!("Failed to load config: {e}"))
            .unwrap_or_default();

        let channels = init_channels()
            .map_err(|e| debug!("Failed to initialize channels: {e}"))
            .unwrap_or_default();

        let process_dir = config.flox.cache_dir.join("process");
        match std::fs::create_dir_all(&process_dir) {
            Ok(_) => {}
            Err(e) => {
                debug!("Failed to create process dir: {e}");
                return vec![];
            }
        };

        let temp_dir = match TempDir::new_in(process_dir) {
            Ok(x) => x,
            Err(e) => {
                debug!("Failed to create temp_dir: {e}");
                return vec![];
            }
        };

        let access_tokens = init::init_access_tokens(&config.nix.access_tokens)
            .map_err(|e| debug!("Failed to initialize access tokens: {e}"))
            .unwrap_or_default();

        let netrc_file = dirs::home_dir()
            .expect("User must have a home directory")
            .join(".netrc");

        let flox = Flox {
            collect_metrics: false,
            cache_dir: config.flox.cache_dir,
            data_dir: config.flox.data_dir,
            config_dir: config.flox.config_dir,
            channels,
            temp_dir: temp_dir.path().to_path_buf(),
            system: env!("NIX_TARGET_SYSTEM").to_string(),
            netrc_file,
            access_tokens,
        };

        let default_prefixes = Self::DEFAULT_PREFIXES;
        let default_flakerefs = Self::DEFAULT_FLAKEREFS;

        let inst = inst.clone();
        let handle = tokio::runtime::Handle::current();
        let comp = std::thread::spawn(move || {
            handle
                .block_on(complete_installable(
                    &flox,
                    &inst,
                    default_flakerefs,
                    default_prefixes,
                ))
                .map_err(|e| debug!("Failed to complete installable: {e}"))
                .unwrap_or_default()
        })
        .join()
        .unwrap();

        comp.into_iter().map(|a| (a, None)).collect()
    }
}

pub async fn complete_installable(
    flox: &Flox,
    installable_str: &String,
    default_flakerefs: &[&str],
    default_attr_prefixes: &[(&str, bool)],
) -> Result<Vec<String>> {
    let mut flox_installables: Vec<FloxInstallable> = vec![];

    if installable_str != "." {
        let trimmed = installable_str.trim_end_matches(|c| c == '.' || c == '#');

        if let Ok(flox_installable) = trimmed.parse() {
            flox_installables.push(flox_installable);
        }

        match trimmed.rsplit_once(|c| c == '.' || c == '#') {
            Some((s, _)) if s != trimmed => flox_installables.push(s.parse()?),
            None => flox_installables.push("".parse()?),
            Some(_) => {}
        };
    } else {
        flox_installables.push(FloxInstallable {
            source: Some(".".to_string()),
            attr_path: vec![],
        });
    };

    let matches = flox
        .resolve_matches(
            flox_installables.as_slice(),
            default_flakerefs,
            default_attr_prefixes,
            true,
        )
        .await?;

    let mut flakerefs: HashSet<String> = HashSet::new();
    let mut prefixes: HashSet<String> = HashSet::new();

    for m in &matches {
        flakerefs.insert(m.flakeref.to_string());
        prefixes.insert(m.prefix.to_string());
    }

    let mut completions: Vec<String> = matches
        .iter()
        .map(|m| {
            let nix_safe_key = m
                .key
                .iter()
                .map(|s| nix_str_safe(s.as_str()))
                .collect::<Vec<_>>()
                .join(".");

            let mut t = vec![format!(
                "{}#{}.{}",
                m.flakeref,
                nix_str_safe(&m.prefix),
                nix_safe_key
            )];

            if let (true, Some(system)) = (m.explicit_system, &m.system) {
                t.push(format!(
                    "{}#{}.{}.{}",
                    m.flakeref,
                    nix_str_safe(&m.prefix),
                    nix_str_safe(&system),
                    nix_safe_key
                ));

                if flakerefs.len() <= 1 {
                    t.push(format!(
                        "{}.{}.{}",
                        nix_str_safe(&m.prefix),
                        nix_str_safe(&system),
                        nix_safe_key
                    ));
                }
            }

            if flakerefs.len() <= 1 && prefixes.len() <= 1 {
                t.push(nix_safe_key.clone());
            }

            if prefixes.len() <= 1 {
                t.push(format!("{}#{}", m.flakeref, nix_safe_key));
            }

            if flakerefs.len() <= 1 {
                t.push(format!("{}.{}", nix_str_safe(&m.prefix), nix_safe_key));
            }

            t
        })
        .flatten()
        .filter(|c| c.starts_with(installable_str))
        .collect();

    completions.sort();
    completions.dedup();

    Ok(completions)
}

pub async fn resolve_installable_from_matches(
    subcommand: &str,
    derivation_type: &str,
    mut matches: Vec<ResolvedInstallableMatch>,
) -> Result<Installable> {
    if matches.len() > 1 {
        // Create set of used prefixes and flakerefs to determine how many are in use
        let mut flakerefs: HashSet<String> = HashSet::new();
        let mut prefixes: HashSet<String> = HashSet::new();

        // Populate the flakerefs and prefixes sets
        for m in &matches {
            flakerefs.insert(m.flakeref.to_string());
            prefixes.insert(m.prefix.to_string());
        }

        // Complile a list of choices for the user to choose from
        let choices: Vec<String> = matches
            .iter()
            .map(
                // Format the results according to how verbose we have to be for disambiguation, only showing the flakeref or prefix when multiple are used
                |m| {
                    let nix_safe_key = m
                        .key
                        .iter()
                        .map(|s| nix_str_safe(s.as_str()))
                        .collect::<Vec<_>>()
                        .join(".");

                    match (flakerefs.len() > 1, prefixes.len() > 1) {
                        (false, false) => nix_safe_key,
                        (true, false) => {
                            format!("{}#{}", m.flakeref, nix_safe_key)
                        }
                        (true, true) => {
                            format!(
                                "{}#{}.{}",
                                m.flakeref,
                                nix_str_safe(&m.prefix),
                                nix_safe_key
                            )
                        }
                        (false, true) => {
                            format!("{}.{}", nix_str_safe(&m.prefix), nix_safe_key)
                        }
                    }
                },
            )
            .collect();

        if !std::io::stderr().is_tty() {
            return Err(anyhow!(
                indoc! {"
                You must address a specific {derivation_type}. For example with:

                    $ flox {subcommand} {first_choice},

                The available packages are:
                {choices_list}
            "},
                derivation_type = derivation_type,
                subcommand = subcommand,
                first_choice = choices.get(0).expect("Expected at least one choice"),
                choices_list = choices
                    .iter()
                    .map(|choice| format!("  - {choice}"))
                    .join("\n")
            ))
            .context(format!(
                "No terminal to prompt for {derivation_type} choice"
            ));
        }

        // Prompt for the user to select match
        let sel = inquire::Select::new(
            &format!("Select a {} for flox {}", derivation_type, subcommand),
            choices,
        )
        .with_flox_theme()
        .raw_prompt()
        .with_context(|| format!("Failed to prompt for {} choice", derivation_type))?;

        let installable = matches.remove(sel.index).installable();

        warn!(
            "HINT: avoid selecting a {} next time with:",
            derivation_type
        );
        warn!(
            "$ flox {} {}",
            subcommand,
            shell_escape::escape(sel.value.into())
        );

        Ok(installable)
    } else if matches.len() == 1 {
        Ok(matches.remove(0).installable())
    } else {
        bail!("No matching installables found");
    }
}
