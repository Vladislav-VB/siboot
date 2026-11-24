//! Step 7 for whatever was installed: hand it this iPhone's pairing, the way
//! SideInstaller does for the apps it knows (`PairingTargets.swift`).
//!
//! Which app it is goes by the signed bundle, not by what was picked in the
//! menu, so an IPA of SideStore picked as a file is handed over like the one
//! siboot downloads:
//!
//! - **SideInstaller** gets the file where it keeps its own, marked as
//!   imported (seed.rs).
//! - **SideStore**, and the SideStore inside **LiveContainer**, get it as
//!   `ALTPairingFile.mobiledevicepairing`, which its older builds read, and as
//!   one file per protocol with the two settings its own import writes, which
//!   its nightlies from 20 September 2026 on read (`SideStorePairingHandoff`).
//!   Its anisette settings are set the way SideInstaller sets them
//!   (`SideStoreAnisetteHandoff`).
//! - **StikDebug, Feather** and the other apps in iLoader's table get the file
//!   where each reads it.
//! - Anything else needs no pairing, and gets nothing.
//!
//! siboot's pairing file holds the RPPairing record only: from iOS 26.4 on,
//! lockdownd hangs up on lockdown records through LocalDevVPN, so no app could
//! use one, and SideInstaller hands the RPPairing record on its own there too.

use std::path::Path;

use idevice::afc::opcode::AfcFopenMode;
use idevice::afc::AfcClient;
use idevice::house_arrest::HouseArrestClient;
use idevice::RsdService;

use crate::fail::{chain, Fail, Result};
use crate::tunnel::Connection;
use crate::{seed, ui};

/// What step 7 does for the app that was installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    SideInstaller,
    /// SideStore, on its own or as LiveContainer's built-in one.
    SideStore(Home),
    /// One file, at this path under the app's Documents.
    File {
        app: &'static str,
        path: &'static str,
    },
    /// No pairing to hand over.
    Nothing,
}

/// Where SideStore lives (`SideStoreHome`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Home {
    Standalone,
    /// Documents/SideStore in LiveContainer's container.
    LiveContainer,
}

impl Home {
    /// SideStore's Documents, from the container's root.
    fn documents(self) -> &'static str {
        match self {
            Home::Standalone => "/Documents",
            Home::LiveContainer => "/Documents/SideStore/Documents",
        }
    }

    /// SideStore's settings, from the container's root. LiveContainer keeps a
    /// guest's settings in its home folder, under the guest's own bundle id.
    fn preferences(self, bundle_id: &str) -> String {
        match self {
            Home::Standalone => format!("/Library/Preferences/{bundle_id}.plist"),
            Home::LiveContainer => {
                "/Documents/SideStore/Library/Preferences/com.SideStore.SideStore.plist".into()
            }
        }
    }
}

/// The apps of iLoader's table that read a pairing file from their Documents,
/// by the name they show, and where: SideInstaller's `PairingTargetApp.all`,
/// less SideStore and LiveContainer (handed over in full above) and Madeira
/// (which takes its record another way).
const FILE_TARGETS: [(&str, &str); 11] = [
    ("Feather", "pairingFile.plist"),
    ("StikDebug", "pairingFile.plist"),
    ("StikTest", "stiktest_pairing.plist"),
    ("Protokolle", "pairingFile.plist"),
    ("Antrag", "pairingFile.plist"),
    ("SparseBox", "pairingFile.plist"),
    ("StikStore", "pairingFile.plist"),
    ("ByeTunes", "pairing file/pairingFile.plist"),
    ("Reynard", "pairingFile.plist"),
    ("PanicAnalyzer", "pairingFile.plist"),
    // Sideloaded StikDebug reads its own path.
    (SIDELOADED_STIKDEBUG, "rp_pairing_file.plist"),
];
const SIDELOADED_STIKDEBUG: &str = "StikDebug (Sideloaded)";

/// The file SideStore read before it kept one per protocol, and LiveContainer's
/// built-in SideStore still reads.
const LEGACY_FILE: &str = "ALTPairingFile.mobiledevicepairing";
/// SideStore's file for the RPPairing record (`SideStorePairingHandoff`).
const REMOTE_FILE: &str = "PairingFile_RemoteRP.plist";
/// The keys of an RPPairing record, `alt_irk` optional.
const REMOTE_KEYS: [&str; 4] = ["private_key", "public_key", "identifier", "alt_irk"];
const REQUIRED_REMOTE_KEYS: [&str; 3] = ["private_key", "public_key", "identifier"];

/// "SideStore (New)" on SideStore's list, and SideStore's own default Mac.
const SIDESTORE_ANISETTE: &str = "https://new.ani.sidestore.io";
const SIDESTORE_CLIENT_INFO: &str =
    "<MacBookPro18,3> <macOS;26.6;25F84> <com.apple.AuthKit/1 (com.apple.akd/1.0)>";
const SIDESTORE_USER_AGENT: &str = "AuthKit/1 (Macintosh; OS X 26.6) (com.apple.akd/1.0)";
/// Servers that run Apple's macOS library, and lillq's pool: a pick of one
/// of these is kept (`AnisetteServer.macADIHosts`, `poolHosts`).
const KEPT_ANISETTE_HOSTS: [&str; 7] = [
    "new.ani.sidestore.io",
    "mac-ani.jkcoxson.com",
    "ani.iloader.app",
    "ani3.stikstore.app",
    "ani.jkcoxson.com",
    "ani2.stikstore.app",
    "anisette.lillq.me",
];

/// What the app signed as `bundle_id`, showing `name` on the Home Screen, is
/// handed.
pub fn target(bundle_id: &str, name: &str) -> Target {
    let id = bundle_id.to_ascii_lowercase();
    if id.starts_with("com.frizzle.sideinstaller") || name == "SideInstaller" {
        return Target::SideInstaller;
    }
    if id.starts_with("com.sidestore.sidestore") || name == "SideStore" {
        return Target::SideStore(Home::Standalone);
    }
    if name == "LiveContainer" || id.starts_with("com.kdt.livecontainer") {
        return Target::SideStore(Home::LiveContainer);
    }
    // The sideloaded StikDebug's entry is reached only through its bundle id.
    let lookup = match name {
        "StikDebug" if id.contains("com.stik.stikdebug") => SIDELOADED_STIKDEBUG,
        SIDELOADED_STIKDEBUG => return Target::Nothing,
        _ => name,
    };
    FILE_TARGETS
        .iter()
        .find(|(app, _)| *app == lookup)
        .map_or(Target::Nothing, |(app, path)| Target::File { app, path })
}

/// The step's title for `target`.
pub fn title(target: Target, name: &str) -> String {
    match target {
        Target::Nothing => format!("Finishing up {name}"),
        _ => format!("Handing {name} the pairing"),
    }
}

/// Step 7. Returns its summary.
pub async fn hand_over(
    connection: &mut Connection,
    target: Target,
    bundle_id: &str,
    name: &str,
    pairing_path: &Path,
) -> Result<String> {
    match target {
        Target::Nothing => {
            ui::ok(&format!("{name} doesn't read a pairing file, so there's nothing to hand it"));
            Ok("nothing to hand over".into())
        }
        Target::SideInstaller => {
            seed::hand_over(connection, bundle_id, pairing_path).await?;
            Ok(format!("{name} has the pairing"))
        }
        Target::File { app, path } => {
            let pairing = read_pairing(pairing_path)?;
            let mut afc = container(connection, bundle_id, name).await?;
            let remote = format!("/Documents/{path}");
            ui::activity(&format!("writing the pairing file into {name}"));
            make_parents(&mut afc, &remote).await;
            let written = write_verified(&mut afc, &remote, &pairing).await?;
            ui::ok(&format!(
                "pairing file written where {app} reads it ({written} bytes, read back and checked)"
            ));
            ui::detail(&remote);
            Ok(format!("{name} has the pairing"))
        }
        Target::SideStore(home) => {
            let pairing = read_pairing(pairing_path)?;
            let mut afc = container(connection, bundle_id, name).await?;
            side_store(&mut afc, home, bundle_id, name, &pairing).await?;
            Ok(format!("{name} has the pairing"))
        }
    }
}

fn read_pairing(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path)
        .map_err(|e| Fail::new(format!("Couldn't read {}.", path.display()), e.to_string()))
}

/// The app's whole container: house_arrest vends it for an app signed for
/// development.
async fn container(connection: &mut Connection, bundle_id: &str, name: &str) -> Result<AfcClient> {
    ui::activity(&format!("opening {name}'s container"));
    let house_arrest =
        HouseArrestClient::connect_rsd(&mut connection.adapter, &mut connection.handshake)
            .await
            .map_err(|e| Fail::new("Couldn't reach house_arrest on this iPhone.", chain(&e)))?;
    house_arrest.vend_container(bundle_id).await.map_err(|e| {
        Fail::new(
            format!(
                "Couldn't open {name}'s container to hand it the pairing. {name} is installed: \
                 import the pairing file in it instead, from ~/.siboot/rp_pairing_file.plist in \
                 Minis."
            ),
            chain(&e),
        )
    })
}

/// SideStore's hand-off. The legacy file is what must arrive; the rest only
/// warns, as in SideInstaller, since older builds don't read it.
async fn side_store(
    afc: &mut AfcClient,
    home: Home,
    bundle_id: &str,
    name: &str,
    pairing: &[u8],
) -> Result<()> {
    let documents = home.documents();
    let legacy = format!("{documents}/{LEGACY_FILE}");
    ui::activity(&format!("writing the pairing file into {name}"));
    make_parents(afc, &legacy).await;
    let written = write_verified(afc, &legacy, pairing).await?;
    ui::ok(&format!(
        "pairing file written into {name} ({written} bytes, read back and checked)"
    ));
    ui::detail(&legacy);

    let prefs_path = home.preferences(bundle_id);
    match remote_record(pairing) {
        Ok(Some(record)) => {
            let remote = format!("{documents}/{REMOTE_FILE}");
            let split = async {
                write_verified(afc, &remote, &record).await?;
                update_plist(afc, &prefs_path, |prefs| pairing_settings(prefs, "rppairing")).await
            };
            match split.await {
                Ok(()) => ui::ok("and as the RPPairing file its newer builds read, set to load it"),
                Err(f) => {
                    ui::warn(&format!(
                        "Couldn't hand {name} its pairing the way SideStore nightlies from 20 \
                         September 2026 on read it. If {name} asks for a pairing file, import \
                         ~/.siboot/rp_pairing_file.plist in it."
                    ));
                    ui::detail(&f.raw);
                }
            }
        }
        Ok(None) => ui::detail("the pairing file has no RPPairing record to split out"),
        Err(raw) => ui::detail(&format!("couldn't read the pairing file's records: {raw}")),
    }

    match update_plist(afc, &prefs_path, anisette_settings).await {
        Ok(changes) if changes.is_empty() => {}
        Ok(changes) => {
            ui::ok(&format!("{name}'s anisette set as SideInstaller sets it"));
            ui::detail(&format!("{} ({prefs_path})", changes.join(", ")));
        }
        Err(f) => {
            ui::warn(&format!(
                "Couldn't set {name}'s anisette. If its sign-in fails with -80046, turn off \
                 On-device Anisette and Auto Rotation in its settings and pick “SideStore (New)”."
            ));
            ui::detail(&f.raw);
        }
    }
    Ok(())
}

/// The RPPairing record alone, as an XML plist (SideStore reads the file as
/// UTF-8), or None when the pairing file has none.
fn remote_record(pairing: &[u8]) -> std::result::Result<Option<Vec<u8>>, String> {
    let dict: plist::Dictionary = plist::from_bytes(pairing).map_err(|e| e.to_string())?;
    if !REQUIRED_REMOTE_KEYS.iter().all(|k| dict.contains_key(k)) {
        return Ok(None);
    }
    let mut record = plist::Dictionary::new();
    for key in REMOTE_KEYS {
        if let Some(value) = dict.get(key) {
            record.insert(key.into(), value.clone());
        }
    }
    let mut out = Vec::new();
    plist::to_writer_xml(&mut out, &record).map_err(|e| e.to_string())?;
    Ok(Some(out))
}

/// What SideStore's own import sets (`SideStorePairingHandoff.applySettings`).
fn pairing_settings(prefs: &mut plist::Dictionary, protocol: &str) {
    prefs.insert("isPairingReset".into(), false.into());
    prefs.insert("activePairingProtocol".into(), protocol.into());
}

/// `SideStoreAnisetteHandoff.applySettings`: what changed, for the log.
fn anisette_settings(prefs: &mut plist::Dictionary) -> Vec<String> {
    let mut changes = Vec::new();
    if prefs.get("useOnDeviceAnisette").and_then(|v| v.as_boolean()) != Some(false) {
        prefs.insert("useOnDeviceAnisette".into(), false.into());
        changes.push("on-device anisette off".into());
    }
    if prefs.get("disableAnisetteRotation").and_then(|v| v.as_boolean()) != Some(true) {
        prefs.insert("disableAnisetteRotation".into(), true.into());
        changes.push("auto-rotation off".into());
    }
    let picked = prefs
        .get("menuAnisetteURL")
        .and_then(|v| v.as_string())
        .and_then(|url| reqwest::Url::parse(url).ok())
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .unwrap_or_default();
    if !KEPT_ANISETTE_HOSTS.contains(&picked.as_str()) {
        prefs.insert("menuAnisetteURL".into(), SIDESTORE_ANISETTE.into());
        changes.push("server “SideStore (New)”".into());
    }
    let saved = prefs
        .get("customAnisetteClientInfo")
        .and_then(|v| v.as_string())
        .unwrap_or("");
    if !saved.is_empty() && !saved.contains("<macOS;") && !saved.contains("<Mac OS X;") {
        prefs.insert("customAnisetteClientInfo".into(), SIDESTORE_CLIENT_INFO.into());
        prefs.insert("customAnisetteUserAgent".into(), SIDESTORE_USER_AGENT.into());
        changes.push("client info back to SideStore's default Mac".into());
    }
    changes
}

/// Edit the plist at `path` with `edit`, keeping what's there, written beside
/// it and renamed over it. Returns what `edit` returns.
async fn update_plist<T>(
    afc: &mut AfcClient,
    path: &str,
    edit: impl FnOnce(&mut plist::Dictionary) -> T,
) -> Result<T> {
    let mut prefs = match read_if_present(afc, path).await? {
        Some(bytes) if !bytes.is_empty() => plist::from_bytes::<plist::Dictionary>(&bytes)
            .map_err(|e| Fail::new(format!("{path} isn't a plist."), e.to_string()))?,
        _ => plist::Dictionary::new(),
    };
    let out = edit(&mut prefs);
    let mut bytes = Vec::new();
    plist::to_writer_binary(&mut bytes, &prefs)
        .map_err(|e| Fail::new(format!("Couldn't encode {path}."), e.to_string()))?;
    make_parents(afc, path).await;
    let staging = format!("{path}.siboot");
    write_verified(afc, &staging, &bytes).await?;
    afc.rename(staging, path)
        .await
        .map_err(|e| Fail::new(format!("Couldn't save {path}."), chain(&e)))?;
    Ok(out)
}

/// Every folder above `path`, made where missing.
async fn make_parents(afc: &mut AfcClient, path: &str) {
    for dir in parents(path) {
        let _ = afc.mk_dir(dir).await;
    }
}

/// "/a", "/a/b" for "/a/b/c".
fn parents(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = path[at + 1..].find('/') {
        at += i + 1;
        out.push(path[..at].to_string());
    }
    out
}

async fn read_if_present(afc: &mut AfcClient, path: &str) -> Result<Option<Vec<u8>>> {
    if afc.get_file_info(path).await.is_err() {
        return Ok(None);
    }
    let mut file = afc
        .open(path, AfcFopenMode::RdOnly)
        .await
        .map_err(|e| Fail::new(format!("Couldn't open {path}."), chain(&e)))?;
    let bytes = file
        .read_entire()
        .await
        .map_err(|e| Fail::new(format!("Couldn't read {path}."), chain(&e)))?;
    let _ = file.close().await;
    Ok(Some(bytes))
}

/// Write `data` to `path`, then read it back and compare. Returns its length.
async fn write_verified(afc: &mut AfcClient, path: &str, data: &[u8]) -> Result<usize> {
    let mut file = afc
        .open(path, AfcFopenMode::WrOnly)
        .await
        .map_err(|e| Fail::new(format!("Couldn't create {path}."), chain(&e)))?;
    file.write_entire(data)
        .await
        .map_err(|e| Fail::new(format!("Couldn't write {path}."), chain(&e)))?;
    file.close()
        .await
        .map_err(|e| Fail::new(format!("Couldn't write {path}."), chain(&e)))?;
    match read_if_present(afc, path).await? {
        Some(back) if back == data => Ok(data.len()),
        Some(back) => Err(Fail::new(
            format!("{path} reads back different from what was written."),
            format!("wrote {} bytes, read {}", data.len(), back.len()),
        )),
        None => Err(Fail::new(format!("{path} vanished after writing."), "")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_signed_bundle_says_what_is_handed_over() {
        assert_eq!(
            target("com.frizzle.sideinstaller.ABCDE12345", "SideInstaller"),
            Target::SideInstaller
        );
        assert_eq!(
            target("com.SideStore.SideStore.ABCDE12345", "SideStore"),
            Target::SideStore(Home::Standalone)
        );
        assert_eq!(
            target("com.kdt.livecontainer.ABCDE12345", "LiveContainer"),
            Target::SideStore(Home::LiveContainer)
        );
        assert_eq!(
            target("com.stik.stikdebug.ABCDE12345", "StikDebug"),
            Target::File {
                app: "StikDebug (Sideloaded)",
                path: "rp_pairing_file.plist"
            }
        );
        assert_eq!(
            target("com.example.stik.ABCDE12345", "StikDebug"),
            Target::File {
                app: "StikDebug",
                path: "pairingFile.plist"
            }
        );
        assert_eq!(
            target("kh.crysalis.feather.ABCDE12345", "Feather"),
            Target::File {
                app: "Feather",
                path: "pairingFile.plist"
            }
        );
        assert_eq!(target("com.example.game.ABCDE12345", "Game"), Target::Nothing);
        // The gated entry is reached only through StikDebug's bundle id.
        assert_eq!(target("x.y", "StikDebug (Sideloaded)"), Target::Nothing);
    }

    #[test]
    fn sidestore_paths_follow_its_home() {
        assert_eq!(Home::Standalone.documents(), "/Documents");
        assert_eq!(
            Home::Standalone.preferences("com.SideStore.SideStore.T"),
            "/Library/Preferences/com.SideStore.SideStore.T.plist"
        );
        assert_eq!(
            Home::LiveContainer.documents(),
            "/Documents/SideStore/Documents"
        );
        assert_eq!(
            Home::LiveContainer.preferences("com.kdt.livecontainer.T"),
            "/Documents/SideStore/Library/Preferences/com.SideStore.SideStore.plist"
        );
    }

    #[test]
    fn parents_of_a_path() {
        assert_eq!(parents("/a/b/c.plist"), ["/a", "/a/b"]);
        assert_eq!(parents("/c.plist"), Vec::<String>::new());
        assert_eq!(
            parents("/Documents/pairing file/pairingFile.plist"),
            ["/Documents", "/Documents/pairing file"]
        );
    }

    fn rp_pairing() -> Vec<u8> {
        let mut dict = plist::Dictionary::new();
        dict.insert("private_key".into(), plist::Value::Data(vec![1; 32]));
        dict.insert("public_key".into(), plist::Value::Data(vec![2; 32]));
        dict.insert("identifier".into(), "HOST-ID".into());
        dict.insert("alt_irk".into(), plist::Value::Data(vec![3; 16]));
        let mut out = Vec::new();
        plist::to_writer_xml(&mut out, &dict).unwrap();
        out
    }

    #[test]
    fn the_rppairing_record_is_split_out_as_xml() {
        let mut merged: plist::Dictionary = plist::from_bytes(&rp_pairing()).unwrap();
        merged.insert("HostID".into(), "LOCKDOWN".into());
        let mut bytes = Vec::new();
        plist::to_writer_binary(&mut bytes, &merged).unwrap();
        let record = remote_record(&bytes).unwrap().unwrap();
        assert!(record.starts_with(b"<?xml"));
        let back: plist::Dictionary = plist::from_bytes(&record).unwrap();
        let mut keys: Vec<&str> = back.keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, ["alt_irk", "identifier", "private_key", "public_key"]);

        let mut lockdown_only = plist::Dictionary::new();
        lockdown_only.insert("HostID".into(), "LOCKDOWN".into());
        let mut bytes = Vec::new();
        plist::to_writer_xml(&mut bytes, &lockdown_only).unwrap();
        assert_eq!(remote_record(&bytes).unwrap(), None);
        assert!(remote_record(b"not a plist").is_err());
    }

    #[test]
    fn sidestore_settings_are_its_own_imports() {
        let mut prefs = plist::Dictionary::new();
        prefs.insert("preferredPairingProtocol".into(), "lockdown".into());
        pairing_settings(&mut prefs, "rppairing");
        assert_eq!(prefs.get("isPairingReset").and_then(|v| v.as_boolean()), Some(false));
        assert_eq!(
            prefs.get("activePairingProtocol").and_then(|v| v.as_string()),
            Some("rppairing")
        );
        // What its own settings picked stays.
        assert_eq!(
            prefs.get("preferredPairingProtocol").and_then(|v| v.as_string()),
            Some("lockdown")
        );
    }

    #[test]
    fn sidestore_anisette_is_set_as_sideinstaller_sets_it() {
        let mut prefs = plist::Dictionary::new();
        let changes = anisette_settings(&mut prefs);
        assert_eq!(changes.len(), 3, "{changes:?}");
        assert_eq!(
            prefs.get("menuAnisetteURL").and_then(|v| v.as_string()),
            Some(SIDESTORE_ANISETTE)
        );
        // Set already: nothing changes the second time.
        assert!(anisette_settings(&mut prefs).is_empty());

        // A Mac-ADI server picked is kept; Android client info goes.
        let mut prefs = plist::Dictionary::new();
        prefs.insert("useOnDeviceAnisette".into(), false.into());
        prefs.insert("disableAnisetteRotation".into(), true.into());
        prefs.insert("menuAnisetteURL".into(), "https://ani.iloader.app/".into());
        prefs.insert(
            "customAnisetteClientInfo".into(),
            "<Pixel> <Android;14> <x>".into(),
        );
        let changes = anisette_settings(&mut prefs);
        assert_eq!(changes, ["client info back to SideStore's default Mac"]);
        assert_eq!(
            prefs.get("menuAnisetteURL").and_then(|v| v.as_string()),
            Some("https://ani.iloader.app/")
        );
        assert_eq!(
            prefs.get("customAnisetteUserAgent").and_then(|v| v.as_string()),
            Some(SIDESTORE_USER_AGENT)
        );
    }
}
