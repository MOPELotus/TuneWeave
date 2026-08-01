use serde::Deserialize;
use std::{collections::BTreeSet, fs, path::Path};

const KNOWN_PLATFORMS: [&str; 8] = [
    "uni", "netease", "qq", "bilibili", "kugou", "migu", "kuwo", "soda",
];

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum CatalogStatus {
    Draft,
    Complete,
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum AuthenticationRequirement {
    None,
    Optional,
    Required,
    Transaction,
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum DestructiveLevel {
    ReadOnly,
    LocalWrite,
    RemoteReversible,
    RemoteDestructive,
    CredentialMutation,
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum NetworkRequirement {
    None,
    Anonymous,
    Authenticated,
    Destructive,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
enum PlatformScope {
    Independent,
    Fixed { platforms: Vec<String> },
    RequestSelected { platforms: Vec<String> },
    ResourceSelected { platforms: Vec<String> },
    CrossPlatform { platforms: Vec<String> },
}

impl PlatformScope {
    fn platforms(&self) -> Option<&[String]> {
        match self {
            Self::Independent => None,
            Self::Fixed { platforms }
            | Self::RequestSelected { platforms }
            | Self::ResourceSelected { platforms }
            | Self::CrossPlatform { platforms } => Some(platforms),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptanceEntry {
    id: String,
    method: String,
    path: String,
    platform_scope: PlatformScope,
    authentication: AuthenticationRequirement,
    destructive_level: DestructiveLevel,
    network: NetworkRequirement,
    acceptance_cases: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptanceCatalog {
    schema_version: u32,
    status: CatalogStatus,
    routes_source: String,
    route_count: usize,
    covered_route_count: usize,
    entries: Vec<AcceptanceEntry>,
}

#[test]
fn acceptance_catalog_is_valid_and_tracks_the_route_denominator() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let routes_path = root.join("docs/acceptance/routes.json");
    let routes: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&routes_path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", routes_path.display())),
    )
    .unwrap_or_else(|error| panic!("invalid {}: {error}", routes_path.display()));
    let route_count = routes["route_count"]
        .as_u64()
        .and_then(|count| usize::try_from(count).ok())
        .expect("route inventory count should be a usize");
    let registered = routes["routes"]
        .as_array()
        .expect("route inventory should contain routes")
        .iter()
        .map(|route| {
            format!(
                "{} {}",
                route["method"]
                    .as_str()
                    .expect("route method should be a string"),
                route["path"]
                    .as_str()
                    .expect("route path should be a string")
            )
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(registered.len(), route_count);

    let catalog_path = root.join("docs/acceptance/endpoints.json");
    let catalog: AcceptanceCatalog = serde_json::from_str(
        &fs::read_to_string(&catalog_path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", catalog_path.display())),
    )
    .unwrap_or_else(|error| panic!("invalid {}: {error}", catalog_path.display()));
    let test_sources = workspace_rust_sources(&root);

    assert_eq!(catalog.schema_version, 1);
    assert_eq!(catalog.routes_source, "docs/acceptance/routes.json");
    assert_eq!(catalog.route_count, route_count);
    assert_eq!(catalog.covered_route_count, catalog.entries.len());
    assert_eq!(
        catalog.status == CatalogStatus::Complete,
        catalog.covered_route_count == catalog.route_count,
        "only a fully covered catalog may use complete status"
    );

    let mut ids = BTreeSet::new();
    let mut covered = BTreeSet::new();
    for entry in &catalog.entries {
        assert!(is_stable_identifier(&entry.id), "invalid id: {}", entry.id);
        assert!(ids.insert(entry.id.as_str()), "duplicate id: {}", entry.id);
        assert!(
            matches!(
                entry.method.as_str(),
                "GET" | "POST" | "PUT" | "PATCH" | "DELETE"
            ),
            "invalid method on {}: {}",
            entry.id,
            entry.method
        );
        let key = format!("{} {}", entry.method, entry.path);
        assert!(
            registered.contains(&key),
            "unknown endpoint on {}: {key}",
            entry.id
        );
        assert!(
            covered.insert(key.clone()),
            "duplicate endpoint entry: {key}"
        );

        validate_platform_scope(entry);
        assert!(
            !entry.acceptance_cases.is_empty(),
            "{} must name at least one acceptance case",
            entry.id
        );
        let mut cases = BTreeSet::new();
        for case in &entry.acceptance_cases {
            assert!(
                is_stable_identifier(case),
                "invalid case on {}: {case}",
                entry.id
            );
            assert!(cases.insert(case), "duplicate case on {}: {case}", entry.id);
            assert!(
                test_sources.contains(&format!("fn {case}(")),
                "{} references an unknown Rust test case: {case}",
                entry.id
            );
        }

        if matches!(
            entry.destructive_level,
            DestructiveLevel::RemoteReversible
                | DestructiveLevel::RemoteDestructive
                | DestructiveLevel::CredentialMutation
        ) {
            assert_ne!(
                entry.network,
                NetworkRequirement::None,
                "remote mutation {} must declare a network requirement",
                entry.id
            );
        }
        if matches!(
            entry.network,
            NetworkRequirement::Authenticated | NetworkRequirement::Destructive
        ) {
            assert_ne!(
                entry.authentication,
                AuthenticationRequirement::None,
                "authenticated network case {} must require or accept credentials",
                entry.id
            );
        }
    }
}

fn validate_platform_scope(entry: &AcceptanceEntry) {
    let Some(platforms) = entry.platform_scope.platforms() else {
        return;
    };
    assert!(
        !platforms.is_empty(),
        "{} has an empty platform scope",
        entry.id
    );
    if matches!(entry.platform_scope, PlatformScope::CrossPlatform { .. }) {
        assert!(
            platforms.len() >= 2,
            "{} cross-platform scope needs at least two platforms",
            entry.id
        );
    }
    let mut unique = BTreeSet::new();
    for platform in platforms {
        assert!(
            KNOWN_PLATFORMS.contains(&platform.as_str()),
            "{} uses unknown platform {platform}",
            entry.id
        );
        assert!(
            unique.insert(platform),
            "{} repeats platform {platform}",
            entry.id
        );
    }
}

fn is_stable_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn workspace_rust_sources(root: &Path) -> String {
    let mut files = Vec::new();
    collect_rust_files(&root.join("crates"), &mut files)
        .expect("workspace Rust files should be enumerable");
    files.sort();
    files
        .into_iter()
        .map(|file| {
            fs::read_to_string(&file)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", file.display()))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn collect_rust_files(
    directory: &Path,
    files: &mut Vec<std::path::PathBuf>,
) -> std::io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, files)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
    Ok(())
}
