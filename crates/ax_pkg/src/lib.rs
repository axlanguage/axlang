use std::fs;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackManifest {
    pub name: String,
    pub version: String,
    pub syntax: Vec<String>,
    pub operations: Vec<String>,
    pub effects: Vec<String>,
    pub native: Vec<String>,
    pub source: String,
}

impl PackManifest {
    pub fn local_native_sources(&self) -> Vec<PathBuf> {
        if self.source.starts_with("http://") || self.source.starts_with("https://") {
            return Vec::new();
        }
        let source = Path::new(&self.source);
        let base = source.parent().unwrap_or_else(|| Path::new("."));
        self.native
            .iter()
            .map(|entry| {
                let path = Path::new(entry);
                if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    base.join(path)
                }
            })
            .collect()
    }

    pub fn resolve_native_sources(&self) -> Result<Vec<PathBuf>, PkgError> {
        if !is_remote_url(&self.source) {
            return Ok(self.local_native_sources());
        }
        let mut sources = Vec::new();
        for entry in &self.native {
            validate_remote_native_entry(entry, &self.source)?;
            let url = remote_native_source_url(&self.source, entry);
            let text = http_get(&url)?;
            let path = self.remote_native_cache_path(entry);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&path, text)?;
            sources.push(path);
        }
        Ok(sources)
    }

    fn remote_native_cache_path(&self, entry: &str) -> PathBuf {
        let mut path = PathBuf::from(".ax-out")
            .join("pack-native")
            .join(sanitize_path_segment(&self.name))
            .join(sanitize_path_segment(&self.version));
        for segment in native_cache_segments(entry) {
            path.push(segment);
        }
        path
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistrySource {
    Builtin,
    Directory(PathBuf),
    Http(String),
}

#[derive(Debug)]
pub enum PkgError {
    Io(io::Error),
    InvalidRegistry(String),
    HttpRegistry {
        url: String,
        message: String,
    },
    UnknownPack {
        name: String,
        registry: String,
        available: Vec<String>,
    },
    InvalidManifest {
        source: String,
        message: String,
    },
}

impl std::fmt::Display for PkgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PkgError::Io(err) => write!(f, "{}", err),
            PkgError::InvalidRegistry(message) => write!(f, "invalid registry: {}", message),
            PkgError::HttpRegistry { url, message } => {
                write!(f, "registry request failed for {}: {}", url, message)
            }
            PkgError::UnknownPack {
                name,
                registry,
                available,
            } => {
                write!(f, "unknown pack `{}` in registry {}", name, registry)?;
                if !available.is_empty() {
                    write!(f, "; available: {}", available.join(", "))?;
                }
                Ok(())
            }
            PkgError::InvalidManifest { source, message } => {
                write!(f, "invalid pack manifest {}: {}", source, message)
            }
        }
    }
}

impl std::error::Error for PkgError {}

impl From<io::Error> for PkgError {
    fn from(value: io::Error) -> Self {
        PkgError::Io(value)
    }
}

pub fn init_project(name: &str) -> std::io::Result<()> {
    let root = Path::new(name);
    fs::create_dir_all(root.join("src"))?;
    fs::write(
        root.join("ax.toml"),
        format!(
            "[package]\nname = \"{}\"\nversion = \"0.1.0\"\n\n[dependencies]\nstd.io = \"1.0\"\n",
            name
        ),
    )?;
    fs::write(root.join("src/main.ax"), "{;\"hello world\"}\n")?;
    Ok(())
}

pub fn add_pack(pack: &str, manifest_path: &Path) -> Result<PackManifest, PkgError> {
    add_pack_from_registry(pack, manifest_path, &RegistrySource::Builtin)
}

pub fn add_pack_from_registry(
    pack: &str,
    manifest_path: &Path,
    registry: &RegistrySource,
) -> Result<PackManifest, PkgError> {
    let pack_manifest = resolve_pack(pack, registry)?;
    let mut text = if manifest_path.exists() {
        fs::read_to_string(manifest_path)?
    } else {
        "[package]\nname = \"ax-app\"\nversion = \"0.1.0\"\n\n[dependencies]\n".to_string()
    };
    let dep_line = format!(
        "{} = \"{}\"",
        pack_manifest.name,
        dependency_version(&pack_manifest.version)
    );
    if has_dependency(&text, &pack_manifest.name) {
        return Ok(pack_manifest);
    }
    if !text.contains("[dependencies]") {
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str("\n[dependencies]\n");
    }
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&dep_line);
    text.push('\n');
    fs::write(manifest_path, text)?;
    Ok(pack_manifest)
}

pub fn read_project_dependencies(manifest_path: &Path) -> Result<Vec<String>, PkgError> {
    if !manifest_path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(manifest_path)?;
    Ok(parse_dependencies(&text))
}

pub fn built_in_registry_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../std")
}

pub fn resolve_builtin_pack(pack: &str) -> Result<PackManifest, PkgError> {
    resolve_pack(pack, &RegistrySource::Builtin)
}

pub fn resolve_pack(pack: &str, registry: &RegistrySource) -> Result<PackManifest, PkgError> {
    if let RegistrySource::Http(base) = registry {
        return resolve_http_pack(pack, base);
    }
    let manifests = list_packs(registry)?;
    manifests
        .iter()
        .find(|manifest| manifest.name == pack)
        .cloned()
        .ok_or_else(|| PkgError::UnknownPack {
            name: pack.to_string(),
            registry: registry.label(),
            available: manifests
                .into_iter()
                .map(|manifest| manifest.name)
                .collect(),
        })
}

pub fn list_builtin_packs() -> Result<Vec<PackManifest>, PkgError> {
    let root = built_in_registry_root();
    match list_directory_packs(&root) {
        Ok(packs) if !packs.is_empty() => Ok(packs),
        Ok(_) | Err(_) => embedded_builtin_packs(),
    }
}

pub fn list_packs(registry: &RegistrySource) -> Result<Vec<PackManifest>, PkgError> {
    match registry {
        RegistrySource::Builtin => list_builtin_packs(),
        RegistrySource::Directory(path) => list_directory_packs(path),
        RegistrySource::Http(base) => list_http_packs(base),
    }
}

pub fn registry_from_value(value: Option<&str>) -> Result<RegistrySource, PkgError> {
    let Some(value) = value else {
        return Ok(RegistrySource::Builtin);
    };
    let value = value.trim();
    if value.is_empty() || value == "builtin" {
        Ok(RegistrySource::Builtin)
    } else if let Some(path) = value.strip_prefix("file://") {
        Ok(RegistrySource::Directory(PathBuf::from(path)))
    } else if value.starts_with("http://") {
        Ok(RegistrySource::Http(
            value.trim_end_matches('/').to_string(),
        ))
    } else if value.starts_with("https://") {
        Ok(RegistrySource::Http(
            value.trim_end_matches('/').to_string(),
        ))
    } else {
        Ok(RegistrySource::Directory(PathBuf::from(value)))
    }
}

pub fn registry_from_env_or_default() -> Result<RegistrySource, PkgError> {
    match std::env::var("AX_REGISTRY") {
        Ok(value) => registry_from_value(Some(&value)),
        Err(std::env::VarError::NotPresent) => Ok(RegistrySource::Builtin),
        Err(err) => Err(PkgError::InvalidRegistry(err.to_string())),
    }
}

impl RegistrySource {
    pub fn label(&self) -> String {
        match self {
            RegistrySource::Builtin => built_in_registry_root().display().to_string(),
            RegistrySource::Directory(path) => path.display().to_string(),
            RegistrySource::Http(base) => base.clone(),
        }
    }
}

fn list_directory_packs(registry_root: &Path) -> Result<Vec<PackManifest>, PkgError> {
    let mut packs = Vec::new();
    for entry in fs::read_dir(&registry_root)? {
        let entry = entry?;
        let manifest_path = entry.path().join("pack.axpack");
        if manifest_path.exists() {
            packs.push(read_pack_manifest(&manifest_path)?);
        }
    }
    packs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(packs)
}

fn read_pack_manifest(path: &Path) -> Result<PackManifest, PkgError> {
    let text = fs::read_to_string(path)?;
    parse_pack_manifest(&text, &path.display().to_string())
}

fn embedded_builtin_packs() -> Result<Vec<PackManifest>, PkgError> {
    BUILTIN_PACK_MANIFESTS
        .iter()
        .map(|(source, text)| parse_pack_manifest(text, source))
        .collect()
}

const BUILTIN_PACK_MANIFESTS: &[(&str, &str)] = &[
    (
        "builtin:std.cli",
        include_str!("../../../std/cli/pack.axpack"),
    ),
    (
        "builtin:std.crypto",
        include_str!("../../../std/crypto/pack.axpack"),
    ),
    (
        "builtin:std.env",
        include_str!("../../../std/env/pack.axpack"),
    ),
    (
        "builtin:std.fs",
        include_str!("../../../std/fs/pack.axpack"),
    ),
    (
        "builtin:std.io",
        include_str!("../../../std/io/pack.axpack"),
    ),
    (
        "builtin:std.json",
        include_str!("../../../std/json/pack.axpack"),
    ),
    (
        "builtin:std.map",
        include_str!("../../../std/map/pack.axpack"),
    ),
    (
        "builtin:std.net.http",
        include_str!("../../../std/net.http/pack.axpack"),
    ),
    (
        "builtin:std.net.http.client",
        include_str!("../../../std/net.http.client/pack.axpack"),
    ),
    (
        "builtin:std.net.tcp",
        include_str!("../../../std/net.tcp/pack.axpack"),
    ),
    (
        "builtin:std.path",
        include_str!("../../../std/path/pack.axpack"),
    ),
    (
        "builtin:std.process",
        include_str!("../../../std/process/pack.axpack"),
    ),
    (
        "builtin:std.str",
        include_str!("../../../std/str/pack.axpack"),
    ),
    (
        "builtin:std.time",
        include_str!("../../../std/time/pack.axpack"),
    ),
    (
        "builtin:std.url",
        include_str!("../../../std/url/pack.axpack"),
    ),
];

fn parse_pack_manifest(text: &str, source: &str) -> Result<PackManifest, PkgError> {
    let name = read_string_key(text, "name").ok_or_else(|| PkgError::InvalidManifest {
        source: source.to_string(),
        message: "missing `name`".to_string(),
    })?;
    let version = read_string_key(text, "version").ok_or_else(|| PkgError::InvalidManifest {
        source: source.to_string(),
        message: "missing `version`".to_string(),
    })?;
    Ok(PackManifest {
        name,
        version,
        syntax: read_string_array_key(text, "syntax").unwrap_or_default(),
        operations: read_string_array_key(text, "operations").unwrap_or_default(),
        effects: read_string_array_key(text, "effects").unwrap_or_default(),
        native: read_string_array_key(text, "native").unwrap_or_default(),
        source: source.to_string(),
    })
}

fn list_http_packs(base: &str) -> Result<Vec<PackManifest>, PkgError> {
    let url = join_url(base, "index.axpack");
    let text = http_get(&url)?;
    let mut packs = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts = line.split_whitespace().collect::<Vec<_>>();
        if parts.len() != 2 {
            return Err(PkgError::InvalidManifest {
                source: format!("{}:{}", url, idx + 1),
                message: "expected `<pack-name> <version>`".to_string(),
            });
        }
        packs.push(PackManifest {
            name: parts[0].to_string(),
            version: parts[1].to_string(),
            syntax: Vec::new(),
            operations: Vec::new(),
            effects: Vec::new(),
            native: Vec::new(),
            source: url.clone(),
        });
    }
    packs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(packs)
}

fn resolve_http_pack(pack: &str, base: &str) -> Result<PackManifest, PkgError> {
    let manifest_url = join_url(base, &format!("{}/pack.axpack", pack));
    match http_get(&manifest_url) {
        Ok(text) => parse_pack_manifest(&text, &manifest_url),
        Err(manifest_err) => {
            let available: Vec<String> = list_http_packs(base)
                .map(|packs| packs.into_iter().map(|pack| pack.name).collect())
                .unwrap_or_default();
            if available.iter().any(|name| name == pack) {
                Err(manifest_err)
            } else {
                Err(PkgError::UnknownPack {
                    name: pack.to_string(),
                    registry: base.to_string(),
                    available,
                })
            }
        }
    }
}

fn join_url(base: &str, suffix: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        suffix.trim_start_matches('/')
    )
}

fn is_remote_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

fn remote_native_source_url(manifest_url: &str, entry: &str) -> String {
    if is_remote_url(entry) {
        return entry.to_string();
    }
    let base = manifest_url
        .rsplit_once('/')
        .map(|(base, _)| base)
        .unwrap_or(manifest_url);
    join_url(base, entry)
}

fn validate_remote_native_entry(entry: &str, source: &str) -> Result<(), PkgError> {
    if entry.trim().is_empty() {
        return Err(PkgError::InvalidManifest {
            source: source.to_string(),
            message: "native source entries cannot be empty".to_string(),
        });
    }
    if is_remote_url(entry) {
        return Ok(());
    }
    if Path::new(entry).components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        )
    }) {
        return Err(PkgError::InvalidManifest {
            source: source.to_string(),
            message: format!("invalid native source path `{}`", entry),
        });
    }
    Ok(())
}

fn native_cache_segments(entry: &str) -> Vec<String> {
    let candidate = if is_remote_url(entry) {
        entry
            .rsplit('/')
            .find(|part| !part.is_empty())
            .unwrap_or(entry)
    } else {
        entry
    };
    let mut segments = Path::new(candidate)
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => {
                Some(sanitize_path_segment(&value.to_string_lossy()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if segments.is_empty() {
        segments.push(sanitize_path_segment(entry));
    }
    segments
}

fn sanitize_path_segment(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        "_".to_string()
    } else {
        sanitized
    }
}

fn http_get(url: &str) -> Result<String, PkgError> {
    if url.starts_with("https://") {
        return https_get_with_curl(url);
    }
    let parsed = parse_http_url(url)?;
    let mut stream =
        TcpStream::connect((&*parsed.host, parsed.port)).map_err(|err| PkgError::HttpRegistry {
            url: url.to_string(),
            message: err.to_string(),
        })?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(PkgError::Io)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(PkgError::Io)?;
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: ax/1.0\r\nConnection: close\r\n\r\n",
        parsed.path, parsed.host
    );
    stream.write_all(request.as_bytes()).map_err(PkgError::Io)?;
    let mut response = String::new();
    stream.read_to_string(&mut response).map_err(PkgError::Io)?;
    let Some((headers, body)) = response.split_once("\r\n\r\n") else {
        return Err(PkgError::HttpRegistry {
            url: url.to_string(),
            message: "malformed HTTP response".to_string(),
        });
    };
    let status_line = headers.lines().next().unwrap_or_default();
    if !status_line.contains(" 200 ") {
        return Err(PkgError::HttpRegistry {
            url: url.to_string(),
            message: status_line.to_string(),
        });
    }
    Ok(body.to_string())
}

fn https_get_with_curl(url: &str) -> Result<String, PkgError> {
    let mut command = Command::new("curl");
    command.args([
        "--fail",
        "--silent",
        "--show-error",
        "--location",
        "--max-time",
        "10",
    ]);
    if let Ok(ca_bundle) = std::env::var("AX_REGISTRY_CA_BUNDLE") {
        if !ca_bundle.trim().is_empty() {
            command.arg("--cacert").arg(ca_bundle);
        }
    }
    let output = command
        .arg(url)
        .output()
        .map_err(|err| PkgError::HttpRegistry {
            url: url.to_string(),
            message: format!("failed to execute curl: {}", err),
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(PkgError::HttpRegistry {
            url: url.to_string(),
            message: if stderr.is_empty() {
                format!("curl exited with {}", output.status)
            } else {
                stderr
            },
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[derive(Debug, PartialEq, Eq)]
struct ParsedHttpUrl {
    host: String,
    port: u16,
    path: String,
}

fn parse_http_url(url: &str) -> Result<ParsedHttpUrl, PkgError> {
    let rest = url.strip_prefix("http://").ok_or_else(|| {
        PkgError::InvalidRegistry(format!("unsupported URL `{}`; expected http://", url))
    })?;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    if authority.is_empty() {
        return Err(PkgError::InvalidRegistry(format!(
            "missing host in URL `{}`",
            url
        )));
    }
    let (host, port) = if let Some((host, port)) = authority.rsplit_once(':') {
        let port = port
            .parse::<u16>()
            .map_err(|_| PkgError::InvalidRegistry(format!("invalid port in URL `{}`", url)))?;
        (host.to_string(), port)
    } else {
        (authority.to_string(), 80)
    };
    Ok(ParsedHttpUrl {
        host,
        port,
        path: format!("/{}", path),
    })
}

fn read_string_key(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        let (lhs, rhs) = line.split_once('=')?;
        if lhs.trim() != key {
            return None;
        }
        let rhs = rhs.trim();
        Some(rhs.strip_prefix('"')?.strip_suffix('"')?.to_string())
    })
}

fn read_string_array_key(text: &str, key: &str) -> Option<Vec<String>> {
    text.lines().find_map(|line| {
        let line = line.trim();
        let (lhs, rhs) = line.split_once('=')?;
        if lhs.trim() != key {
            return None;
        }
        parse_string_array(rhs.trim())
    })
}

fn parse_string_array(value: &str) -> Option<Vec<String>> {
    let inner = value.strip_prefix('[')?.strip_suffix(']')?.trim();
    if inner.is_empty() {
        return Some(Vec::new());
    }
    let mut values = Vec::new();
    for item in inner.split(',') {
        let item = item.trim();
        values.push(item.strip_prefix('"')?.strip_suffix('"')?.to_string());
    }
    Some(values)
}

fn parse_dependencies(text: &str) -> Vec<String> {
    let mut in_dependencies = false;
    let mut dependencies = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_dependencies = line == "[dependencies]";
            continue;
        }
        if !in_dependencies {
            continue;
        }
        if let Some((name, _)) = line.split_once('=') {
            dependencies.push(name.trim().to_string());
        }
    }
    dependencies
}

fn dependency_version(version: &str) -> String {
    version
        .strip_suffix(".0")
        .map(str::to_string)
        .unwrap_or_else(|| version.to_string())
}

fn has_dependency(text: &str, pack: &str) -> bool {
    text.lines().any(|line| {
        let Some((lhs, _)) = line.split_once('=') else {
            return false;
        };
        lhs.trim() == pack
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_builtin_pack_manifest() {
        let manifest = resolve_builtin_pack("std.net.http").expect("pack");
        assert_eq!(manifest.name, "std.net.http");
        assert_eq!(manifest.version, "1.0.0");
    }

    #[test]
    fn embeds_builtin_pack_manifests_for_released_binaries() {
        let packs = embedded_builtin_packs().expect("embedded packs");
        assert!(packs.iter().any(|pack| pack.name == "std.io"));
        assert!(packs.iter().any(|pack| pack.name == "std.net.tcp"));
        assert!(packs.iter().all(|pack| pack.source.starts_with("builtin:")));
    }

    #[test]
    fn rejects_unknown_pack() {
        let err = resolve_builtin_pack("std.missing").expect_err("unknown");
        assert!(err.to_string().contains("unknown pack `std.missing`"));
    }

    #[test]
    fn resolves_pack_from_directory_registry() {
        let root = unique_temp_dir("ax_pkg_registry");
        let pack_dir = root.join("acme.web");
        fs::create_dir_all(&pack_dir).expect("mkdir");
        fs::write(
            pack_dir.join("pack.axpack"),
            "name = \"acme.web\"\nversion = \"2.1.0\"\nsyntax = []\noperations = [\"web.serve\"]\neffects = []\nnative = [\"native.c\"]\n",
        )
        .expect("write");
        let registry = RegistrySource::Directory(root.clone());
        let manifest = resolve_pack("acme.web", &registry).expect("resolve");
        assert_eq!(manifest.name, "acme.web");
        assert_eq!(manifest.version, "2.1.0");
        assert!(manifest.syntax.is_empty());
        assert_eq!(manifest.operations, vec!["web.serve".to_string()]);
        assert!(manifest.effects.is_empty());
        assert_eq!(manifest.native, vec!["native.c".to_string()]);
        assert_eq!(
            manifest.local_native_sources(),
            vec![pack_dir.join("native.c")]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn parses_pack_manifest_effects() {
        let manifest = parse_pack_manifest(
            "name = \"acme.telemetry\"\nversion = \"1.0.0\"\nsyntax = [\"track\"]\noperations = [\"telemetry.track\"]\neffects = [\"telemetry.write\", \"net.write\"]\nnative = [\"native.c\"]\n",
            "inline",
        )
        .expect("manifest");
        assert_eq!(manifest.syntax, vec!["track".to_string()]);
        assert_eq!(manifest.operations, vec!["telemetry.track".to_string()]);
        assert_eq!(
            manifest.effects,
            vec!["telemetry.write".to_string(), "net.write".to_string()]
        );
        assert_eq!(manifest.native, vec!["native.c".to_string()]);
    }

    #[test]
    fn reads_project_dependencies_from_manifest_text() {
        let dependencies = parse_dependencies(
            "[package]\nname = \"app\"\n\n[dependencies]\nstd.io = \"1.0\"\nacme.web = \"2.1\"\n\n[other]\nnope = \"0\"\n",
        );
        assert_eq!(
            dependencies,
            vec!["std.io".to_string(), "acme.web".to_string()]
        );
    }

    #[test]
    fn parses_http_url_with_port_and_path() {
        let parsed = parse_http_url("http://127.0.0.1:8080/registry").expect("url");
        assert_eq!(
            parsed,
            ParsedHttpUrl {
                host: "127.0.0.1".to_string(),
                port: 8080,
                path: "/registry".to_string()
            }
        );
    }

    #[test]
    fn accepts_https_registry_source() {
        let registry = registry_from_value(Some("https://registry.example.test/ax")).expect("url");
        assert_eq!(
            registry,
            RegistrySource::Http("https://registry.example.test/ax".to_string())
        );
    }

    #[test]
    fn resolves_relative_remote_native_source_url() {
        assert_eq!(
            remote_native_source_url(
                "https://registry.example.test/acme.telemetry/pack.axpack",
                "native/native.c",
            ),
            "https://registry.example.test/acme.telemetry/native/native.c"
        );
    }

    #[test]
    fn rejects_remote_native_path_traversal() {
        let err = validate_remote_native_entry("../native.c", "inline").expect_err("invalid");
        assert!(err.to_string().contains("invalid native source path"));
    }

    #[test]
    fn lists_packs_from_http_registry() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("listen");
        let port = listener.local_addr().expect("addr").port();
        let handle = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buffer = [0_u8; 1024];
                let _ = stream.read(&mut buffer);
                let body = "acme.web 2.1.0\n";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        let registry = RegistrySource::Http(format!("http://127.0.0.1:{}", port));
        let packs = list_packs(&registry).expect("packs");
        handle.join().expect("server");
        assert_eq!(packs[0].name, "acme.web");
        assert_eq!(packs[0].version, "2.1.0");
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "{}_{}_{}",
            prefix,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        path
    }
}
