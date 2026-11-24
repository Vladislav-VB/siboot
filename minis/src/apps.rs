//! What siboot installs. SideInstaller, as it always has, or what's picked
//! from a menu before step 1: SideStore, LiveContainer with SideStore, or an
//! IPA of your own: a file on this iPhone, picked on the page; one from a
//! link; one kept from an earlier run or in a folder mounted in Minis.
//!
//! The menu comes before step 1 because step 1 gets the IPA, while a VPN
//! that reaches GitHub may still be on (download.rs). `--app` and `--ipa`
//! answer it on the command line, and a run with no one to ask installs
//! SideInstaller, as before.

use std::io::IsTerminal as _;
use std::path::Path;

use crate::download::{self, Release};
use crate::fail::{Fail, Result};
use crate::handover::Target;
use crate::team::Renewal;
use crate::{library, ui, web, Args};

/// An app siboot knows where to get.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum App {
    SideInstaller,
    /// SideStore's latest release.
    SideStore,
    /// SideStore's nightly, the build that reads an iOS 27 pairing.
    SideStoreNightly,
    /// LiveContainer, with SideStore built in.
    LiveContainer,
}

impl App {
    /// `--app NAME`.
    pub fn parse(name: &str) -> Option<App> {
        let name: String = name
            .to_ascii_lowercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        match name.as_str() {
            "sideinstaller" => Some(App::SideInstaller),
            "sidestore" | "sidestorestable" => Some(App::SideStore),
            "sidestorenightly" | "nightly" => Some(App::SideStoreNightly),
            "livecontainer" | "livecontainersidestore" | "lc" => Some(App::LiveContainer),
            _ => None,
        }
    }

    pub fn release(self) -> &'static Release {
        match self {
            App::SideInstaller => &download::SIDEINSTALLER,
            App::SideStore => &download::SIDESTORE,
            App::SideStoreNightly => &download::SIDESTORE_NIGHTLY,
            App::LiveContainer => &download::LIVECONTAINER,
        }
    }

    pub fn name(self) -> &'static str {
        self.release().name
    }
}

pub const QUESTION: &str = "what should siboot install?";
/// The menu's first choices, in order: `App`s, then an IPA of your own.
pub const CHOICES: [&str; 4] = [
    "SideInstaller",
    "SideStore",
    "LiveContainer + SideStore",
    "An .ipa of your own",
];
const SIDESTORE_QUESTION: &str = "which SideStore?";
const SIDESTORE_CHOICES: [&str; 2] = [
    "Nightly · the one that reads siboot's pairing on iOS 27",
    "Stable · its latest release",
];
const OWN_QUESTION: &str = "which .ipa?";
const PICK: &str = "Choose a file on this iPhone…";
const LINK: &str = "Download one from a link…";
const TYPED: &str = "Type the path of one in Minis…";

/// What this run installs, in `args`: as the command line says, or as the
/// menu is answered. A run that can't ask installs SideInstaller.
pub fn pick(args: &Args) -> Result<Args> {
    if args.app.is_some() || args.ipa.is_some() || !can_ask() {
        return Ok(args.clone());
    }
    let with_app = |app: App| Args {
        app: Some(app),
        ..args.clone()
    };
    loop {
        let choices: Vec<String> = CHOICES.iter().map(|c| c.to_string()).collect();
        match ui::choose(QUESTION, &choices) {
            None => return Err(Fail::by_user("Stopped before picking what to install.")),
            Some(0) => return Ok(with_app(App::SideInstaller)),
            Some(1) => {
                let builds: Vec<String> = SIDESTORE_CHOICES.iter().map(|c| c.to_string()).collect();
                match ui::choose(SIDESTORE_QUESTION, &builds) {
                    None => continue,
                    Some(0) => return Ok(with_app(App::SideStoreNightly)),
                    Some(_) => return Ok(with_app(App::SideStore)),
                }
            }
            Some(2) => return Ok(with_app(App::LiveContainer)),
            Some(_) => {
                if let Some(ipa) = own(&args.home)? {
                    return Ok(Args {
                        ipa: Some(ipa),
                        ..args.clone()
                    });
                }
            }
        }
    }
}

/// Someone to ask: the page, the live screen, or a terminal to type in.
fn can_ask() -> bool {
    ui::interactive() || std::io::stdin().is_terminal()
}

enum Own {
    Pick,
    Link,
    Typed,
    File(String),
}

/// An IPA of your own, as a path or a link; None to go back.
fn own(home: &Path) -> Result<Option<String>> {
    loop {
        let mut options: Vec<(String, Own)> = Vec::new();
        if web::page().is_some() {
            options.push((PICK.into(), Own::Pick));
        }
        options.push((LINK.into(), Own::Link));
        for entry in library::kept(home) {
            let path = entry.path.to_string_lossy().into_owned();
            options.push((entry.label(None), Own::File(path)));
        }
        for entry in library::mounted() {
            let path = entry.path.to_string_lossy().into_owned();
            options.push((entry.label(library::place(&entry).as_deref()), Own::File(path)));
        }
        options.push((TYPED.into(), Own::Typed));
        let labels: Vec<String> = options.iter().map(|(label, _)| label.clone()).collect();
        let Some(i) = ui::choose(OWN_QUESTION, &labels) else {
            return Ok(None);
        };
        match &options[i].1 {
            Own::File(path) => return Ok(Some(path.clone())),
            Own::Pick => {
                let dir = library::dir(home)?;
                let notes = [
                    "It's sent to siboot on this iPhone, and kept in ~/.siboot/library, so it's \
                     here to pick again next time: a free account's apps last 7 days.",
                ];
                // Step 1 says which file it uses.
                if let Some((path, _)) = ui::pick_file("pick the .ipa to install", &notes, &dir) {
                    return Ok(Some(path.to_string_lossy().into_owned()));
                }
            }
            Own::Link => {
                let Some(link) = ui::ask_text(
                    "download the .ipa from which link?",
                    "Link",
                    "https://example.com/App.ipa",
                    "Link to the .ipa: ",
                ) else {
                    continue;
                };
                match link_problem(&link) {
                    None => return Ok(Some(link)),
                    Some(problem) => ui::warn(problem),
                }
            }
            Own::Typed => {
                let Some(path) = ui::ask_text(
                    "which file? Its path in Minis",
                    "Path",
                    "/var/minis/mounts/Folder/App.ipa",
                    "Path to the .ipa: ",
                ) else {
                    continue;
                };
                match path_problem(Path::new(&path)) {
                    None => return Ok(Some(path)),
                    Some(problem) => ui::warn(&problem),
                }
            }
        }
    }
}

/// What's wrong with a link typed for the IPA, if anything.
fn link_problem(link: &str) -> Option<&'static str> {
    let parsed = reqwest::Url::parse(link).ok();
    match parsed {
        Some(url) if matches!(url.scheme(), "https" | "http") && url.host_str().is_some() => None,
        _ => Some("That isn't a link siboot can download from: it starts with https://."),
    }
}

/// What's wrong with a path typed for the IPA, if anything.
fn path_problem(path: &Path) -> Option<String> {
    let mut magic = [0u8; 4];
    let read = std::fs::File::open(path).and_then(|mut f| std::io::Read::read_exact(&mut f, &mut magic));
    match read {
        Ok(()) if &magic == b"PK\x03\x04" => None,
        Ok(()) => Some(format!("{} isn't an IPA.", path.display())),
        Err(e) => Some(format!("Couldn't read {}: {e}.", path.display())),
    }
}

/// How the app installed renews itself, for the finish box.
pub fn renewal(target: Target) -> Renewal {
    match target {
        Target::SideInstaller => Renewal::SideInstaller,
        Target::SideStore(_) => Renewal::SideStore,
        Target::File { .. } | Target::Nothing => Renewal::Siboot,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_names_parse_loosely() {
        assert_eq!(App::parse("SideInstaller"), Some(App::SideInstaller));
        assert_eq!(App::parse("sidestore"), Some(App::SideStore));
        assert_eq!(App::parse("SideStore-Nightly"), Some(App::SideStoreNightly));
        assert_eq!(App::parse("livecontainer+sidestore"), Some(App::LiveContainer));
        assert_eq!(App::parse("LiveContainer"), Some(App::LiveContainer));
        assert_eq!(App::parse("altstore"), None);
        assert_eq!(App::LiveContainer.name(), "LiveContainer + SideStore");
        assert_eq!(App::SideStoreNightly.release().track, download::Track::Nightly);
    }

    #[test]
    fn links_and_paths_are_checked_before_step_1() {
        assert_eq!(link_problem("https://example.com/App.ipa"), None);
        assert_eq!(link_problem("http://192.168.1.2:8000/App.ipa"), None);
        assert!(link_problem("example.com/App.ipa").is_some());
        assert!(link_problem("file:///etc/passwd").is_some());
        assert!(link_problem("https://").is_some());

        let dir = std::env::temp_dir().join(format!("siboot-apps-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let good = dir.join("App.ipa");
        std::fs::write(&good, b"PK\x03\x04rest").unwrap();
        assert_eq!(path_problem(&good), None);
        let page = dir.join("page.ipa");
        std::fs::write(&page, b"<html>").unwrap();
        assert!(path_problem(&page).unwrap().contains("isn't an IPA"));
        assert!(path_problem(&dir.join("missing.ipa")).unwrap().starts_with("Couldn't read"));
    }

    #[test]
    fn what_renews_what() {
        assert_eq!(renewal(Target::SideInstaller), Renewal::SideInstaller);
        assert_eq!(
            renewal(Target::SideStore(crate::handover::Home::LiveContainer)),
            Renewal::SideStore
        );
        assert_eq!(renewal(Target::Nothing), Renewal::Siboot);
    }
}
