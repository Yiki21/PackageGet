use std::{
    collections::{HashMap, HashSet},
    env,
    path::{Path, PathBuf},
    process::Output,
    time::Duration,
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::time::timeout;
use updater_manager_api::{
    AuthorizationHint, ManagerAvailability, ManagerCapabilities, ManagerCapability,
    ManagerCategory, ManagerConfig, ManagerDescriptor, ManagerError, ManagerErrorKind, ManagerId,
    ManagerResult, PackageAction, PackageInfo, PackageManager, PackageOrigin, PackageScope,
    PackageTarget, PackageUpdate, Platform, ProgressEvent, ProgressSink, SupportedPlatforms,
};

use crate::{
    command::{
        CommandSpec, command_status_error, manager_availability, resolve_executable, run_output,
    },
    progress::run_cancellable_command_with_progress,
};

const RUBYGEMS_ID: &str = "builtin:rubygems";
const GEM_COMMAND: &str = "gem";
const ORIGIN_NAME: &str = "RubyGems";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(90);

/// Direct `updater-manager-api` implementation for RubyGems repositories.
#[derive(Debug, Clone)]
pub struct RubyGemsManager {
    descriptor: ManagerDescriptor,
}

impl RubyGemsManager {
    /// Creates the built-in RubyGems manager.
    #[must_use]
    pub fn new() -> Self {
        let descriptor = ManagerDescriptor::new(
            ManagerId::parse(RUBYGEMS_ID).expect("RubyGems manager ID must remain valid"),
            "RubyGems",
            ManagerCategory::Development,
            SupportedPlatforms::from([Platform::Linux, Platform::Windows, Platform::MacOs]),
            ManagerCapabilities::from([
                ManagerCapability::Installed,
                ManagerCapability::Updates,
                ManagerCapability::Search,
                ManagerCapability::Install,
                ManagerCapability::Update,
                ManagerCapability::Uninstall,
            ]),
        )
        .expect("RubyGems descriptor must remain valid")
        .with_description("Ruby packages installed in RubyGems repositories")
        .with_authorization(AuthorizationHint::MayRequireElevation {
            message: Some(
                "Writes to protected GEM_HOME repositories may require authorization.".to_owned(),
            ),
        });
        Self { descriptor }
    }

    fn validate_config(&self, config: &ManagerConfig) -> ManagerResult<()> {
        if &config.id == self.descriptor.id() {
            Ok(())
        } else {
            Err(protocol(
                "RubyGems configuration ID does not match the manager",
                &format!("expected {}, received {}", self.descriptor.id(), config.id),
            ))
        }
    }

    async fn environment(&self, config: &ManagerConfig) -> ManagerResult<GemEnvironment> {
        self.validate_config(config)?;
        let gem = resolve_executable(config, GEM_COMMAND);
        let home = environment_path(&gem, "home").await?;
        let user_home = environment_path(&gem, "user_gemhome").await?;
        let path_output = run_success(
            &gem_command(&gem).args(["environment", "path"]),
            "RubyGems path query timed out",
        )
        .await?;
        let raw_path = decode_utf8(
            &path_output.stdout,
            "RubyGems path response is not valid UTF-8",
        )?;
        let raw_path = single_line(&raw_path, "RubyGems path response is malformed")?;
        let mut repositories = env::split_paths(raw_path)
            .map(|path| validate_repository(path, "RubyGems path contains an invalid repository"))
            .collect::<ManagerResult<Vec<_>>>()?;
        if repositories.is_empty() {
            return Err(protocol("RubyGems path contains no repositories", raw_path));
        }
        if !repositories.contains(&home) {
            repositories.insert(0, home.clone());
        }
        let mut seen = HashSet::new();
        repositories.retain(|repository| seen.insert(repository.clone()));
        Ok(GemEnvironment {
            home,
            user_home,
            repositories,
        })
    }

    async fn installed_entries(
        &self,
        config: &ManagerConfig,
    ) -> ManagerResult<(GemEnvironment, Vec<InstalledGem>)> {
        let environment = self.environment(config).await?;
        let gem = resolve_executable(config, GEM_COMMAND);
        let mut installed = Vec::new();
        for repository in &environment.repositories {
            let output = run_success(
                &repository_command(&gem, repository).args([
                    "list",
                    "--local",
                    "--details",
                    "--all",
                ]),
                "RubyGems installed listing timed out",
            )
            .await?;
            let value = decode_utf8(
                &output.stdout,
                "RubyGems installed listing is not valid UTF-8",
            )?;
            installed.extend(parse_installed(&value, repository, &environment)?);
        }
        reject_duplicate_installed(&installed)?;
        installed.sort_by(|left, right| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
                .then_with(|| left.version.cmp(&right.version))
                .then_with(|| left.repository.cmp(&right.repository))
        });
        Ok((environment, installed))
    }

    fn write_command(
        &self,
        config: &ManagerConfig,
        environment: &GemEnvironment,
        action: PackageAction,
        target: &PackageTarget,
    ) -> ManagerResult<CommandSpec> {
        ensure_supported_action(action)?;
        if &target.manager_id != self.descriptor.id() {
            return Err(protocol(
                "RubyGems target belongs to another manager",
                &target.name,
            ));
        }
        validate_gem_name(&target.name)?;
        let origin = GemReference::parse(target.origin.as_ref().ok_or_else(|| {
            protocol("RubyGems target is missing its typed origin", &target.name)
        })?)?;
        if !environment.repositories.contains(&origin.repository) {
            return Err(protocol(
                "RubyGems target repository is not in the current GEM_PATH",
                &origin.repository.to_string_lossy(),
            ));
        }
        let expected_scope = environment.scope(&origin.repository);
        if target.scope != expected_scope {
            return Err(protocol(
                "RubyGems target scope does not match its repository",
                &target.name,
            ));
        }

        let gem = resolve_executable(config, GEM_COMMAND);
        let repository = origin.repository.to_string_lossy();
        match action {
            PackageAction::Install => {
                if origin.version.is_some() || origin.default {
                    return Err(protocol(
                        "RubyGems install target must use a remote origin",
                        &target.name,
                    ));
                }
                let mut command = repository_command(&gem, &origin.repository).args([
                    "install",
                    target.name.as_str(),
                    "--install-dir",
                    repository.as_ref(),
                    "--no-document",
                ]);
                if let Some(version) = target.version.as_deref() {
                    validate_version(version)?;
                    command = command.args(["--version", version]);
                }
                Ok(command)
            }
            PackageAction::Update => {
                require_installed_origin(&origin, target)?;
                reject_target_version(target)?;
                Ok(repository_command(&gem, &origin.repository).args([
                    "update",
                    target.name.as_str(),
                    "--install-dir",
                    repository.as_ref(),
                    "--no-document",
                ]))
            }
            PackageAction::Uninstall => {
                let version = require_installed_origin(&origin, target)?;
                reject_target_version(target)?;
                if origin.default {
                    return Err(ManagerError::new(
                        ManagerErrorKind::Unsupported,
                        "default Ruby gems cannot be uninstalled",
                    )
                    .with_detail(&target.name));
                }
                Ok(repository_command(&gem, &origin.repository).args([
                    "uninstall",
                    target.name.as_str(),
                    "--install-dir",
                    repository.as_ref(),
                    "--version",
                    version,
                    "--executables",
                    "--abort-on-dependent",
                ]))
            }
            _ => unreachable!("supported actions were checked above"),
        }
    }
}

impl Default for RubyGemsManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageManager for RubyGemsManager {
    fn descriptor(&self) -> &ManagerDescriptor {
        &self.descriptor
    }

    async fn availability(&self, config: &ManagerConfig) -> ManagerResult<ManagerAvailability> {
        self.validate_config(config)?;
        Ok(manager_availability(config, GEM_COMMAND, &["--version"]).await)
    }

    async fn installed(&self, config: &ManagerConfig) -> ManagerResult<Vec<PackageInfo>> {
        let (_, installed) = self.installed_entries(config).await?;
        Ok(installed
            .into_iter()
            .map(|entry| entry.info(self.descriptor.id()))
            .collect())
    }

    async fn count_installed(&self, config: &ManagerConfig) -> ManagerResult<usize> {
        Ok(self.installed_entries(config).await?.1.len())
    }

    async fn updates(
        &self,
        config: &ManagerConfig,
        _refresh: bool,
    ) -> ManagerResult<Vec<PackageUpdate>> {
        let (environment, installed) = self.installed_entries(config).await?;
        let installed_by_identity = installed
            .iter()
            .map(|entry| {
                (
                    (
                        entry.repository.clone(),
                        entry.name.to_ascii_lowercase(),
                        entry.version.clone(),
                    ),
                    entry,
                )
            })
            .collect::<HashMap<_, _>>();
        let gem = resolve_executable(config, GEM_COMMAND);
        let mut updates = Vec::new();
        for repository in &environment.repositories {
            let output = run_success(
                &repository_command(&gem, repository).arg("outdated"),
                "RubyGems outdated query timed out",
            )
            .await?;
            let value = decode_utf8(
                &output.stdout,
                "RubyGems outdated response is not valid UTF-8",
            )?;
            for candidate in parse_outdated(&value)? {
                let key = (
                    repository.clone(),
                    candidate.name.to_ascii_lowercase(),
                    candidate.current.clone(),
                );
                let Some(installed) = installed_by_identity.get(&key) else {
                    let injected_from_another_repository =
                        installed_by_identity.keys().any(|(_, name, version)| {
                            name == &candidate.name.to_ascii_lowercase()
                                && version == &candidate.current
                        });
                    if injected_from_another_repository {
                        continue;
                    }
                    return Err(protocol(
                        "RubyGems outdated entry is absent from installed inventory",
                        &format!("{} {}", candidate.name, candidate.current),
                    ));
                };
                updates.push(PackageUpdate::new(
                    installed.target(self.descriptor.id()),
                    candidate.current,
                    candidate.latest,
                ));
            }
        }
        Ok(updates)
    }

    async fn search(&self, config: &ManagerConfig, query: &str) -> ManagerResult<Vec<PackageInfo>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        validate_search_term(query)?;
        let environment = self.environment(config).await?;
        let gem = resolve_executable(config, GEM_COMMAND);
        let output = run_success(
            &repository_command(&gem, &environment.home)
                .args(["search", query, "--remote", "--all"]),
            "RubyGems remote search timed out",
        )
        .await?;
        let value = decode_utf8(
            &output.stdout,
            "RubyGems remote search response is not valid UTF-8",
        )?;
        parse_search(&value, self.descriptor.id(), &environment)
    }

    async fn execute(
        &self,
        config: &ManagerConfig,
        action: PackageAction,
        packages: &[PackageTarget],
        progress: &dyn ProgressSink,
    ) -> ManagerResult<()> {
        self.validate_config(config)?;
        ensure_supported_action(action)?;
        let environment = self.environment(config).await?;
        let commands = packages
            .iter()
            .map(|target| self.write_command(config, &environment, action, target))
            .collect::<ManagerResult<Vec<_>>>()?;
        let total = packages.len();
        progress.emit(ProgressEvent::Started { action, total });
        for (index, (target, command)) in packages.iter().zip(&commands).enumerate() {
            timeout(
                COMMAND_TIMEOUT,
                run_cancellable_command_with_progress(command, progress, |event| {
                    let (fraction, message) = event.into_parts();
                    if let Some(message) = message {
                        progress.emit(ProgressEvent::Message { message });
                    }
                    progress.emit(ProgressEvent::Advanced {
                        completed: index + usize::from(fraction >= 1.0),
                        total,
                        current_package: Some(target.name.clone()),
                    });
                }),
            )
            .await
            .map_err(|_| {
                ManagerError::new(
                    ManagerErrorKind::Timeout,
                    "RubyGems write command timed out",
                )
                .with_detail(command.program().to_string_lossy())
            })??;
        }
        progress.emit(ProgressEvent::Finished {
            completed: total,
            total,
        });
        Ok(())
    }
}

#[derive(Debug)]
struct GemEnvironment {
    home: PathBuf,
    user_home: PathBuf,
    repositories: Vec<PathBuf>,
}

impl GemEnvironment {
    fn scope(&self, repository: &Path) -> PackageScope {
        if repository == self.user_home {
            PackageScope::User
        } else {
            PackageScope::System
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GemReference {
    repository: PathBuf,
    version: Option<String>,
    default: bool,
}

impl GemReference {
    fn installed(repository: &Path, version: &str, default: bool) -> Self {
        Self {
            repository: repository.to_owned(),
            version: Some(version.to_owned()),
            default,
        }
    }

    fn remote(repository: &Path) -> Self {
        Self {
            repository: repository.to_owned(),
            version: None,
            default: false,
        }
    }

    fn origin(&self) -> PackageOrigin {
        PackageOrigin::new(ORIGIN_NAME).with_reference(
            serde_json::to_string(self).expect("validated RubyGems origin must serialize"),
        )
    }

    fn parse(origin: &PackageOrigin) -> ManagerResult<Self> {
        if origin.name != ORIGIN_NAME {
            return Err(protocol(
                "RubyGems target origin name is malformed",
                &origin.name,
            ));
        }
        let reference = origin
            .reference
            .as_deref()
            .ok_or_else(|| protocol("RubyGems target origin reference is missing", &origin.name))?;
        let parsed: Self = serde_json::from_str(reference).map_err(|error| {
            protocol(
                "RubyGems target origin reference is malformed",
                &error.to_string(),
            )
        })?;
        validate_repository(
            parsed.repository.clone(),
            "RubyGems target repository is malformed",
        )?;
        if let Some(version) = parsed.version.as_deref() {
            validate_version(version)?;
        }
        if parsed.default && parsed.version.is_none() {
            return Err(protocol(
                "RubyGems remote origin cannot be a default gem",
                reference,
            ));
        }
        Ok(parsed)
    }
}

#[derive(Debug)]
struct InstalledGem {
    name: String,
    version: String,
    repository: PathBuf,
    scope: PackageScope,
    default: bool,
}

impl InstalledGem {
    fn target(&self, manager_id: &ManagerId) -> PackageTarget {
        let mut target = PackageTarget::new(manager_id.clone(), &self.name);
        target.scope = self.scope;
        target.origin =
            Some(GemReference::installed(&self.repository, &self.version, self.default).origin());
        target
    }

    fn info(self, manager_id: &ManagerId) -> PackageInfo {
        let mut info = PackageInfo::new(manager_id.clone(), &self.name, &self.version);
        info.description = Some(format!(
            "Repository: {}{}",
            self.repository.display(),
            if self.default { "; default gem" } else { "" }
        ));
        info.scope = self.scope;
        info.origin =
            Some(GemReference::installed(&self.repository, &self.version, self.default).origin());
        info
    }
}

#[derive(Debug)]
struct OutdatedGem {
    name: String,
    current: String,
    latest: String,
}

fn parse_installed(
    value: &str,
    expected_repository: &Path,
    environment: &GemEnvironment,
) -> ManagerResult<Vec<InstalledGem>> {
    let mut installed = Vec::new();
    let mut header: Option<(String, Vec<String>)> = None;
    let mut found_versions = HashSet::new();

    for line in value.lines() {
        if line.trim().is_empty() || line.starts_with("*** ") {
            continue;
        }
        if !line.starts_with(char::is_whitespace) {
            finish_detail_entry(&header, &found_versions)?;
            header = Some(parse_name_versions(line)?);
            found_versions.clear();
            continue;
        }
        let Some((name, versions)) = header.as_ref() else {
            return Err(protocol(
                "RubyGems details contain metadata before a gem header",
                line,
            ));
        };
        let trimmed = line.trim_start();
        let location = if let Some(value) = trimmed.strip_prefix("Installed at") {
            Some(value.trim_start())
        } else if trimmed.starts_with('(') && found_versions.len() < versions.len() {
            Some(trimmed)
        } else {
            None
        };
        let Some(location) = location else {
            continue;
        };
        let (version, default, repository) = parse_location(location, versions)?;
        if !environment.repositories.contains(&repository) {
            return Err(protocol(
                "RubyGems reported a repository outside the current GEM_PATH",
                &repository.to_string_lossy(),
            ));
        }
        if !found_versions.insert(version.clone()) {
            return Err(protocol(
                "RubyGems details contain a duplicate version location",
                &format!("{name} {version}"),
            ));
        }
        if repository == expected_repository {
            installed.push(InstalledGem {
                name: name.clone(),
                version,
                repository,
                scope: environment.scope(expected_repository),
                default,
            });
        }
    }
    finish_detail_entry(&header, &found_versions)?;
    Ok(installed)
}

fn finish_detail_entry(
    header: &Option<(String, Vec<String>)>,
    found_versions: &HashSet<String>,
) -> ManagerResult<()> {
    if let Some((name, versions)) = header
        && (versions.len() != found_versions.len()
            || versions
                .iter()
                .any(|version| !found_versions.contains(version)))
    {
        return Err(protocol(
            "RubyGems details are missing installed version locations",
            name,
        ));
    }
    Ok(())
}

fn parse_location(value: &str, versions: &[String]) -> ManagerResult<(String, bool, PathBuf)> {
    let (label, raw_repository) = value
        .split_once(':')
        .ok_or_else(|| protocol("RubyGems installed location is malformed", value))?;
    let repository = validate_repository(
        PathBuf::from(raw_repository.trim()),
        "RubyGems installed location is malformed",
    )?;
    let label = label.trim();
    if label.is_empty() {
        if versions.len() != 1 {
            return Err(protocol(
                "RubyGems multi-version location is missing its version",
                value,
            ));
        }
        return Ok((versions[0].clone(), false, repository));
    }
    let label = label
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
        .ok_or_else(|| protocol("RubyGems installed location label is malformed", value))?;
    if label == "default" {
        if versions.len() != 1 {
            return Err(protocol(
                "RubyGems default location is missing its version",
                value,
            ));
        }
        return Ok((versions[0].clone(), true, repository));
    }
    let (version, default) = label
        .strip_suffix(", default")
        .map_or((label, false), |version| (version, true));
    validate_version(version)?;
    if !versions.iter().any(|candidate| candidate == version) {
        return Err(protocol(
            "RubyGems installed location version is absent from its header",
            value,
        ));
    }
    Ok((version.to_owned(), default, repository))
}

fn parse_name_versions(value: &str) -> ManagerResult<(String, Vec<String>)> {
    let value = value.trim();
    let (name, versions) = value
        .strip_suffix(')')
        .and_then(|value| value.rsplit_once(" ("))
        .ok_or_else(|| protocol("RubyGems gem/version row is malformed", value))?;
    validate_gem_name(name)?;
    let versions = versions
        .split(',')
        .map(str::trim)
        .map(|version| {
            validate_version(version)?;
            Ok(version.to_owned())
        })
        .collect::<ManagerResult<Vec<_>>>()?;
    if versions.is_empty() {
        return Err(protocol("RubyGems gem/version row has no versions", value));
    }
    Ok((name.to_owned(), versions))
}

fn parse_outdated(value: &str) -> ManagerResult<Vec<OutdatedGem>> {
    let mut outdated = Vec::new();
    for line in value.lines().filter(|line| !line.trim().is_empty()) {
        let (name, comparison) = line
            .trim()
            .strip_suffix(')')
            .and_then(|line| line.rsplit_once(" ("))
            .ok_or_else(|| protocol("RubyGems outdated row is malformed", line))?;
        validate_gem_name(name)?;
        let (current, latest) = comparison
            .split_once(" < ")
            .ok_or_else(|| protocol("RubyGems outdated comparison is malformed", line))?;
        validate_version(current)?;
        validate_version(latest)?;
        outdated.push(OutdatedGem {
            name: name.to_owned(),
            current: current.to_owned(),
            latest: latest.to_owned(),
        });
    }
    let mut seen = HashSet::new();
    if let Some(duplicate) = outdated
        .iter()
        .map(|entry| entry.name.to_ascii_lowercase())
        .find(|name| !seen.insert(name.clone()))
    {
        return Err(protocol(
            "RubyGems outdated response contains a duplicate gem",
            &duplicate,
        ));
    }
    Ok(outdated)
}

fn parse_search(
    value: &str,
    manager_id: &ManagerId,
    environment: &GemEnvironment,
) -> ManagerResult<Vec<PackageInfo>> {
    let mut packages = Vec::new();
    for line in value
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with("*** "))
    {
        let line = line.trim();
        let (name, versions) = line
            .strip_suffix(')')
            .and_then(|line| line.rsplit_once(" ("))
            .ok_or_else(|| protocol("RubyGems search row is malformed", line))?;
        validate_gem_name(name)?;
        let latest = versions
            .split(',')
            .next()
            .and_then(|version| version.split_whitespace().next())
            .ok_or_else(|| protocol("RubyGems search result has no latest version", line))?;
        validate_version(latest)?;
        let mut info = PackageInfo::new(manager_id.clone(), name, latest);
        info.scope = environment.scope(&environment.home);
        info.origin = Some(GemReference::remote(&environment.home).origin());
        packages.push(info);
    }
    let mut seen = HashSet::new();
    if let Some(duplicate) = packages
        .iter()
        .map(|entry| entry.name.to_ascii_lowercase())
        .find(|name| !seen.insert(name.clone()))
    {
        return Err(protocol(
            "RubyGems search response contains a duplicate gem",
            &duplicate,
        ));
    }
    Ok(packages)
}

fn reject_duplicate_installed(installed: &[InstalledGem]) -> ManagerResult<()> {
    let mut seen = HashSet::new();
    for entry in installed {
        let identity = (
            entry.repository.clone(),
            entry.name.to_ascii_lowercase(),
            entry.version.clone(),
        );
        if !seen.insert(identity) {
            return Err(protocol(
                "RubyGems inventory contains a duplicate installed identity",
                &format!("{} {}", entry.name, entry.version),
            ));
        }
    }
    Ok(())
}

fn require_installed_origin<'a>(
    origin: &'a GemReference,
    target: &PackageTarget,
) -> ManagerResult<&'a str> {
    origin.version.as_deref().ok_or_else(|| {
        protocol(
            "RubyGems installed operation requires an exact origin version",
            &target.name,
        )
    })
}

fn reject_target_version(target: &PackageTarget) -> ManagerResult<()> {
    if target.version.is_some() {
        Err(ManagerError::new(
            ManagerErrorKind::Unsupported,
            "version-pinned RubyGems update/uninstall targets are not supported",
        )
        .with_detail(&target.name))
    } else {
        Ok(())
    }
}

fn validate_gem_name(value: &str) -> ManagerResult<()> {
    let valid = !value.is_empty()
        && value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        });
    if valid {
        Ok(())
    } else {
        Err(protocol("RubyGems gem name is malformed", value))
    }
}

fn validate_version(value: &str) -> ManagerResult<()> {
    if value.is_empty()
        || value.starts_with('-')
        || value.chars().any(char::is_whitespace)
        || value.contains(['/', '\\', ';', '='])
    {
        Err(protocol("RubyGems version is malformed", value))
    } else {
        Ok(())
    }
}

fn validate_search_term(value: &str) -> ManagerResult<()> {
    if value.starts_with('-') || value.contains(['\n', '\r', '\0']) {
        Err(protocol("RubyGems search term is malformed", value))
    } else {
        Ok(())
    }
}

fn validate_repository(path: PathBuf, message: &str) -> ManagerResult<PathBuf> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(protocol(message, &path.to_string_lossy()))
    }
}

fn ensure_supported_action(action: PackageAction) -> ManagerResult<()> {
    match action {
        PackageAction::Install | PackageAction::Update | PackageAction::Uninstall => Ok(()),
        _ => Err(ManagerError::new(
            ManagerErrorKind::Unsupported,
            "RubyGems package action is unsupported",
        )),
    }
}

fn gem_command(path: &Path) -> CommandSpec {
    CommandSpec::new(path)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env_remove("RUBYGEMS_GEMDEPS")
}

fn repository_command(path: &Path, repository: &Path) -> CommandSpec {
    gem_command(path)
        .env("GEM_HOME", repository.as_os_str())
        .env("GEM_PATH", repository.as_os_str())
}

async fn environment_path(gem: &Path, field: &str) -> ManagerResult<PathBuf> {
    let output = run_success(
        &gem_command(gem).args(["environment", field]),
        "RubyGems environment query timed out",
    )
    .await?;
    let value = decode_utf8(
        &output.stdout,
        "RubyGems environment response is not valid UTF-8",
    )?;
    let value = single_line(&value, "RubyGems environment response is malformed")?;
    validate_repository(
        PathBuf::from(value),
        "RubyGems environment repository is not absolute",
    )
}

fn single_line<'a>(value: &'a str, message: &str) -> ManagerResult<&'a str> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.lines().count() != 1 {
        Err(protocol(message, value))
    } else {
        Ok(trimmed)
    }
}

async fn run_success(spec: &CommandSpec, timeout_message: &str) -> ManagerResult<Output> {
    let output = timeout(COMMAND_TIMEOUT, run_output(spec))
        .await
        .map_err(|_| {
            ManagerError::new(ManagerErrorKind::Timeout, timeout_message)
                .with_detail(spec.program().to_string_lossy())
        })??;
    if output.status.success() {
        return Ok(output);
    }
    Err(command_status_error(
        spec,
        output.status,
        &format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    ))
}

fn decode_utf8(bytes: &[u8], message: &str) -> ManagerResult<String> {
    String::from_utf8(bytes.to_vec()).map_err(|error| protocol(message, &error.to_string()))
}

fn protocol(message: &str, detail: &str) -> ManagerError {
    ManagerError::new(ManagerErrorKind::Protocol, message).with_detail(detail)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    fn environment(repository: &str, user: &str) -> GemEnvironment {
        GemEnvironment {
            home: PathBuf::from(repository),
            user_home: PathBuf::from(user),
            repositories: vec![PathBuf::from(repository), PathBuf::from(user)],
        }
    }

    #[test]
    fn details_parser_preserves_multiple_versions_and_default_state() {
        let repository = if cfg!(windows) { r"C:\gems" } else { "/gems" };
        let user = if cfg!(windows) {
            r"C:\user-gems"
        } else {
            "/user-gems"
        };
        let value = format!(
            "rake (13.0.6, 12.3.3)\n    Installed at (13.0.6): {repository}\n                 (12.3.3, default): {repository}\n"
        );
        let parsed = parse_installed(
            &value,
            Path::new(repository),
            &environment(repository, user),
        )
        .expect("parse detailed inventory");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].version, "13.0.6");
        assert!(!parsed[0].default);
        assert_eq!(parsed[1].version, "12.3.3");
        assert!(parsed[1].default);
    }

    #[test]
    fn details_parser_rejects_missing_and_external_locations() {
        let repository = if cfg!(windows) { r"C:\gems" } else { "/gems" };
        let user = if cfg!(windows) {
            r"C:\user-gems"
        } else {
            "/user-gems"
        };
        let env = environment(repository, user);
        assert!(parse_installed("rake (1.0)\n", Path::new(repository), &env).is_err());
        let external = if cfg!(windows) { r"C:\other" } else { "/other" };
        let value = format!("rake (1.0)\n    Installed at: {external}\n");
        assert!(parse_installed(&value, Path::new(repository), &env).is_err());
    }

    #[test]
    fn outdated_and_search_reject_duplicate_identity() {
        assert!(parse_outdated("rake (1.0 < 2.0)\nRAKE (1.1 < 2.0)\n").is_err());
        let repository = if cfg!(windows) { r"C:\gems" } else { "/gems" };
        let user = if cfg!(windows) {
            r"C:\user-gems"
        } else {
            "/user-gems"
        };
        let manager_id = ManagerId::parse(RUBYGEMS_ID).expect("manager ID");
        assert!(
            parse_search(
                "rake (2.0)\nRAKE (1.0)\n",
                &manager_id,
                &environment(repository, user)
            )
            .is_err()
        );
    }

    #[test]
    fn search_accepts_platform_qualified_remote_versions() {
        let repository = if cfg!(windows) { r"C:\gems" } else { "/gems" };
        let user = if cfg!(windows) {
            r"C:\user-gems"
        } else {
            "/user-gems"
        };
        let manager_id = ManagerId::parse(RUBYGEMS_ID).expect("manager ID");
        let parsed = parse_search(
            "nokogiri (1.18.10 ruby, 1.18.10 x86_64-linux-gnu)\n",
            &manager_id,
            &environment(repository, user),
        )
        .expect("platform-qualified search output");
        assert_eq!(parsed[0].version, "1.18.10");
    }

    #[test]
    fn gem_commands_disable_project_dependency_discovery() {
        let command = gem_command(Path::new(GEM_COMMAND));
        assert!(
            command
                .removed_environment()
                .iter()
                .any(|name| name == OsStr::new("RUBYGEMS_GEMDEPS"))
        );
    }
}
