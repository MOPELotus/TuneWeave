use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

const INVENTORY_SCHEMA_VERSION: u32 = 1;
const INVENTORY_SOURCE: &str = "crates/tuneweave-server/src/lib.rs";

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
struct RouteInventory {
    schema_version: u32,
    source: String,
    route_count: usize,
    routes: Vec<RouteSpec>,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
struct RouteSpec {
    method: String,
    path: String,
}

#[test]
fn checked_in_route_inventory_matches_router() {
    let source_path = source_path();
    let source = fs::read_to_string(&source_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", source_path.display()));
    let actual = inventory_from_source(&source).expect("router source should be extractable");
    let inventory_path = inventory_path();

    if std::env::var_os("TUNEWEAVE_UPDATE_ENDPOINT_INVENTORY").is_some() {
        let parent = inventory_path
            .parent()
            .expect("inventory path should have a parent");
        fs::create_dir_all(parent)
            .unwrap_or_else(|error| panic!("failed to create {}: {error}", parent.display()));
        let mut encoded =
            serde_json::to_string_pretty(&actual).expect("inventory should serialize as JSON");
        encoded.push('\n');
        fs::write(&inventory_path, encoded).unwrap_or_else(|error| {
            panic!("failed to write {}: {error}", inventory_path.display())
        });
        return;
    }

    let encoded = fs::read_to_string(&inventory_path).unwrap_or_else(|error| {
        panic!(
            "failed to read {}: {error}; set TUNEWEAVE_UPDATE_ENDPOINT_INVENTORY=1 and run \
             cargo test -p tuneweave-server --test endpoint_inventory",
            inventory_path.display()
        )
    });
    let expected: RouteInventory = serde_json::from_str(&encoded)
        .unwrap_or_else(|error| panic!("invalid {}: {error}", inventory_path.display()));

    assert_eq!(
        expected, actual,
        "route inventory drifted; set TUNEWEAVE_UPDATE_ENDPOINT_INVENTORY=1 and run \
         cargo test -p tuneweave-server --test endpoint_inventory"
    );
}

#[test]
fn extracts_multiline_routes_and_chained_methods() {
    let source = r#"
pub fn build_router(state: AppState) -> Router {
    let versioned = Router::new()
        .route("/tracks", get(tracks_get).post(tracks_post))
        .route(
            "/playlists/{reference}",
            get(playlist).patch(playlist_update).delete(playlist_delete),
        );

    Router::new()
        .route("/healthz", get(health))
        .nest("/v1", versioned)
        .with_state(state)
}
"#;

    let inventory = inventory_from_source(source).expect("fixture should be extractable");

    assert_eq!(
        inventory.routes,
        vec![
            RouteSpec {
                method: "GET".to_owned(),
                path: "/healthz".to_owned(),
            },
            RouteSpec {
                method: "DELETE".to_owned(),
                path: "/v1/playlists/{reference}".to_owned(),
            },
            RouteSpec {
                method: "GET".to_owned(),
                path: "/v1/playlists/{reference}".to_owned(),
            },
            RouteSpec {
                method: "PATCH".to_owned(),
                path: "/v1/playlists/{reference}".to_owned(),
            },
            RouteSpec {
                method: "GET".to_owned(),
                path: "/v1/tracks".to_owned(),
            },
            RouteSpec {
                method: "POST".to_owned(),
                path: "/v1/tracks".to_owned(),
            },
        ]
    );
}

#[test]
fn rejects_duplicate_and_unsupported_route_methods() {
    let duplicate = r#"
pub fn build_router(state: AppState) -> Router {
    let versioned = Router::new()
        .route("/tracks", get(tracks))
        .route("/tracks", get(other_tracks));

    Router::new()
        .route("/healthz", get(health))
        .nest("/v1", versioned)
        .with_state(state)
}
"#;
    let unsupported = r#"
pub fn build_router(state: AppState) -> Router {
    let versioned = Router::new()
        .route("/tracks", get(tracks).head(track_headers));

    Router::new()
        .route("/healthz", get(health))
        .nest("/v1", versioned)
        .with_state(state)
}
"#;

    assert_eq!(
        inventory_from_source(duplicate),
        Err("duplicate route GET /v1/tracks in build_router".to_owned())
    );
    assert_eq!(
        inventory_from_source(unsupported),
        Err("unsupported Axum routing constructor head in route call".to_owned())
    );
}

fn source_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs")
}

fn inventory_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/acceptance/routes.json")
}

fn inventory_from_source(source: &str) -> Result<RouteInventory, String> {
    let function = extract_function(source, "pub fn build_router")?;
    let versioned_start = function
        .find("let versioned = Router::new()")
        .ok_or_else(|| "build_router is missing the versioned Router".to_owned())?;
    let root_marker = "\n\n    Router::new()";
    let root_start = function[versioned_start..]
        .find(root_marker)
        .map(|index| versioned_start + index + 2)
        .ok_or_else(|| "build_router is missing the root Router".to_owned())?;

    let mut routes = extract_routes(&function[versioned_start..root_start], "/v1")?;
    routes.extend(extract_routes(&function[root_start..], "")?);
    routes.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.method.cmp(&right.method))
    });

    let mut unique = HashSet::with_capacity(routes.len());
    for route in &routes {
        if !unique.insert((route.method.as_str(), route.path.as_str())) {
            return Err(format!(
                "duplicate route {} {} in build_router",
                route.method, route.path
            ));
        }
    }

    Ok(RouteInventory {
        schema_version: INVENTORY_SCHEMA_VERSION,
        source: INVENTORY_SOURCE.to_owned(),
        route_count: routes.len(),
        routes,
    })
}

fn extract_function<'a>(source: &'a str, signature: &str) -> Result<&'a str, String> {
    let signature_start = source
        .find(signature)
        .ok_or_else(|| format!("source is missing {signature}"))?;
    let brace = source[signature_start..]
        .find('{')
        .map(|index| signature_start + index)
        .ok_or_else(|| format!("{signature} is missing its body"))?;
    let end = matching_delimiter(source, brace, b'{', b'}')?;
    Ok(&source[brace + 1..end])
}

fn extract_routes(source: &str, prefix: &str) -> Result<Vec<RouteSpec>, String> {
    if source.contains(".route_service(") {
        return Err("route_service is not supported by the inventory extractor".to_owned());
    }

    let mut routes = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = source[cursor..].find(".route(") {
        let open = cursor + relative + ".route".len();
        let close = matching_delimiter(source, open, b'(', b')')?;
        let call = &source[open + 1..close];
        let path = first_string_literal(call)?;
        let methods = routing_methods(call)?;
        if methods.is_empty() {
            return Err(format!("route {path} has no recognized HTTP method"));
        }

        for method in methods {
            routes.push(RouteSpec {
                method: method.to_owned(),
                path: format!("{prefix}{path}"),
            });
        }
        cursor = close + 1;
    }

    Ok(routes)
}

fn matching_delimiter(
    source: &str,
    open: usize,
    opening: u8,
    closing: u8,
) -> Result<usize, String> {
    let bytes = source.as_bytes();
    if bytes.get(open) != Some(&opening) {
        return Err(format!("expected delimiter at byte {open}"));
    }

    let mut depth = 1_usize;
    let mut index = open + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => index = skip_string(bytes, index)?,
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = skip_block_comment(bytes, index)?;
            }
            byte if byte == opening => {
                depth += 1;
                index += 1;
            }
            byte if byte == closing => {
                depth -= 1;
                if depth == 0 {
                    return Ok(index);
                }
                index += 1;
            }
            _ => index += 1,
        }
    }

    Err(format!("unclosed delimiter at byte {open}"))
}

fn skip_string(bytes: &[u8], quote: usize) -> Result<usize, String> {
    let mut index = quote + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'"' => return Ok(index + 1),
            _ => index += 1,
        }
    }
    Err(format!("unclosed string at byte {quote}"))
}

fn skip_block_comment(bytes: &[u8], start: usize) -> Result<usize, String> {
    let mut depth = 1_usize;
    let mut index = start + 2;
    while index + 1 < bytes.len() {
        match (bytes[index], bytes[index + 1]) {
            (b'/', b'*') => {
                depth += 1;
                index += 2;
            }
            (b'*', b'/') => {
                depth -= 1;
                index += 2;
                if depth == 0 {
                    return Ok(index);
                }
            }
            _ => index += 1,
        }
    }
    Err(format!("unclosed block comment at byte {start}"))
}

fn first_string_literal(call: &str) -> Result<String, String> {
    let bytes = call.as_bytes();
    let start = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .ok_or_else(|| "empty route call".to_owned())?;
    if bytes[start] != b'"' {
        return Err("route path must be a string literal".to_owned());
    }
    let end = skip_string(bytes, start)? - 1;
    let path = &call[start + 1..end];
    if path.contains('\\') {
        return Err(format!("escaped route path is not supported: {path}"));
    }
    if !path.starts_with('/') {
        return Err(format!("route path must start with '/': {path}"));
    }
    Ok(path.to_owned())
}

fn routing_methods(call: &str) -> Result<Vec<&'static str>, String> {
    let bytes = call.as_bytes();
    let mut methods = Vec::new();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'"' {
            index = skip_string(bytes, index).unwrap_or(bytes.len());
            continue;
        }
        if !is_identifier_start(bytes[index]) {
            index += 1;
            continue;
        }

        let start = index;
        index += 1;
        while index < bytes.len() && is_identifier_continue(bytes[index]) {
            index += 1;
        }
        let identifier = &call[start..index];
        let mut next = index;
        while next < bytes.len() && bytes[next].is_ascii_whitespace() {
            next += 1;
        }
        if bytes.get(next) != Some(&b'(') {
            continue;
        }

        let method = match identifier {
            "get" => Some("GET"),
            "post" => Some("POST"),
            "put" => Some("PUT"),
            "patch" => Some("PATCH"),
            "delete" => Some("DELETE"),
            "any" | "any_service" | "connect" | "head" | "on" | "on_service" | "options"
            | "trace" => {
                return Err(format!(
                    "unsupported Axum routing constructor {identifier} in route call"
                ));
            }
            _ => None,
        };
        if let Some(method) = method
            && !methods.contains(&method)
        {
            methods.push(method);
        }
    }

    Ok(methods)
}

fn is_identifier_start(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic()
}

fn is_identifier_continue(byte: u8) -> bool {
    is_identifier_start(byte) || byte.is_ascii_digit()
}
