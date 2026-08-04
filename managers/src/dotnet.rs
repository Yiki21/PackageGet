use std::{collections::HashSet, path::Path, process::Output, time::Duration};

use async_trait::async_trait;
use semver::Version;
use serde::Deserialize;
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
    progress::{CommandProgress, run_cancellable_command_with_progress, run_command_with_progress},
};

const DOTNET_ID: &str = "builtin:dotnet-tool";
const DOTNET_COMMAND: &str = "dotnet";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(90);
const SEARCH_LIMIT: &str = "50";

/// Direct `updater-manager-api` implementation for current-user .NET global tools.
#[derive(Debug, Clone)]
pub struct DotnetToolManager {
    descriptor: ManagerDescriptor,
}

impl DotnetToolManager {
    /// Creates the built-in .NET global tools manager.
    #[must_use]
    pub fn new() -> Self {
        let descriptor = ManagerDescriptor::new(
            ManagerId::parse(DOTNET_ID).expect("dotnet tool manager ID must remain valid"),
            ".NET global tools",
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
        .expect("dotnet tool descriptor must remain valid")
        .with_description("Current-user global tools installed with the .NET SDK")
        .with_authorization(AuthorizationHint::None);
        Self { descriptor }
    }

    /// Executes one validated target with bounded command progress.
    ///
    /// # Errors
    ///
    /// Returns a typed target-validation, timeout, or command error.
    #[allow(dead_code)]
    async fn execute_target_with_progress(
        &self,
        config: &ManagerConfig,
        action: PackageAction,
        target: &PackageTarget,
        on_progress: impl FnMut(CommandProgress),
    ) -> ManagerResult<()> {
        self.validate_config(config)?;
        let command = self.write_command(config, action, target)?;
        timeout(
            COMMAND_TIMEOUT,
            run_command_with_progress(&command, on_progress),
        )
        .await
        .map_err(|_| {
            ManagerError::new(
                ManagerErrorKind::Timeout,
                ".NET global tool write command timed out",
            )
            .with_detail(command.program().to_string_lossy())
        })?
    }

    async fn installed_tools(&self, config: &ManagerConfig) -> ManagerResult<Vec<InstalledTool>> {
        self.validate_config(config)?;
        let dotnet = resolve_executable(config, DOTNET_COMMAND);
        let output = run_success(
            &dotnet_command(&dotnet).args(["tool", "list", "--global", "--format", "json"]),
            ".NET global tool listing timed out",
        )
        .await?;
        let value = decode_utf8(
            &output.stdout,
            ".NET global tool listing is not valid UTF-8",
        )?;
        parse_installed(&value)
    }

    async fn latest_version(
        &self,
        config: &ManagerConfig,
        package_id: &str,
    ) -> ManagerResult<String> {
        let dotnet = resolve_executable(config, DOTNET_COMMAND);
        let output = run_success(
            &dotnet_command(&dotnet).args([
                "package",
                "search",
                package_id,
                "--exact-match",
                "--format",
                "json",
            ]),
            ".NET tool NuGet metadata query timed out",
        )
        .await?;
        let value = decode_utf8(
            &output.stdout,
            ".NET tool NuGet metadata is not valid UTF-8",
        )?;
        parse_latest_version(&value, package_id)
    }

    fn write_command(
        &self,
        config: &ManagerConfig,
        action: PackageAction,
        target: &PackageTarget,
    ) -> ManagerResult<CommandSpec> {
        ensure_supported_action(action)?;
        if &target.manager_id != self.descriptor.id() {
            return Err(protocol(
                ".NET global tool target belongs to another manager",
                &target.name,
            ));
        }
        validate_package_id(&target.name)?;
        if target.scope != PackageScope::Unknown || target.origin.is_some() {
            if target.scope != PackageScope::User {
                return Err(ManagerError::new(
                    ManagerErrorKind::Unsupported,
                    ".NET tool target scope is not supported",
                )
                .with_detail(&target.name));
            }
            let origin = target.origin.as_ref().ok_or_else(|| {
                protocol(
                    ".NET global tool target is missing its typed origin",
                    &target.name,
                )
            })?;
            validate_origin(origin, &target.name)?;
        }

        let dotnet = resolve_executable(config, DOTNET_COMMAND);
        let verb = match action {
            PackageAction::Install => "install",
            PackageAction::Update => "update",
            PackageAction::Uninstall => "uninstall",
            _ => unreachable!("supported actions were checked above"),
        };
        let mut command =
            dotnet_command(&dotnet).args(["tool", verb, target.name.as_str(), "--global"]);
        match (action, target.version.as_deref()) {
            (PackageAction::Install, Some(version)) => {
                validate_version(version)?;
                command = command.args(["--version", version]);
            }
            (PackageAction::Update | PackageAction::Uninstall, Some(_)) => {
                return Err(ManagerError::new(
                    ManagerErrorKind::Unsupported,
                    "version-pinned .NET tool operations are not supported",
                )
                .with_detail(&target.name));
            }
            _ => {}
        }
        Ok(command)
    }
}

impl Default for DotnetToolManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageManager for DotnetToolManager {
    fn descriptor(&self) -> &ManagerDescriptor {
        &self.descriptor
    }

    async fn availability(&self, config: &ManagerConfig) -> ManagerResult<ManagerAvailability> {
        self.validate_config(config)?;
        Ok(manager_availability(config, DOTNET_COMMAND, &["--version"]).await)
    }

    async fn installed(&self, config: &ManagerConfig) -> ManagerResult<Vec<PackageInfo>> {
        Ok(self
            .installed_tools(config)
            .await?
            .into_iter()
            .map(|tool| tool.info(self.descriptor.id()))
            .collect())
    }

    async fn count_installed(&self, config: &ManagerConfig) -> ManagerResult<usize> {
        Ok(self.installed_tools(config).await?.len())
    }

    async fn updates(
        &self,
        config: &ManagerConfig,
        _refresh: bool,
    ) -> ManagerResult<Vec<PackageUpdate>> {
        let installed = self.installed_tools(config).await?;
        let mut updates = Vec::new();
        for tool in installed {
            let latest = self.latest_version(config, &tool.package_id).await?;
            let installed_version = Version::parse(&tool.version).map_err(|error| {
                protocol(
                    ".NET installed tool version is not semantic",
                    &format!("{}: {error}", tool.version),
                )
            })?;
            let available_version = Version::parse(&latest).map_err(|error| {
                protocol(
                    ".NET available tool version is not semantic",
                    &format!("{latest}: {error}"),
                )
            })?;
            if available_version != installed_version {
                updates.push(PackageUpdate::new(
                    tool.target(self.descriptor.id()),
                    tool.version,
                    latest,
                ));
            }
        }
        Ok(updates)
    }

    async fn search(&self, config: &ManagerConfig, query: &str) -> ManagerResult<Vec<PackageInfo>> {
        self.validate_config(config)?;
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        validate_search_term(query)?;
        let dotnet = resolve_executable(config, DOTNET_COMMAND);
        let output = run_success(
            &dotnet_command(&dotnet).args([
                "tool",
                "search",
                query,
                "--detail",
                "--take",
                SEARCH_LIMIT,
            ]),
            ".NET tool search timed out",
        )
        .await?;
        let value = decode_utf8(&output.stdout, ".NET tool search is not valid UTF-8")?;
        parse_tool_search(&value, self.descriptor.id())
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
        let commands = packages
            .iter()
            .map(|target| self.write_command(config, action, target))
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
                    ".NET global tool write command timed out",
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

#[derive(Debug, Deserialize)]
struct ToolListDocument {
    version: u64,
    data: Vec<ToolListEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolListEntry {
    package_id: String,
    version: String,
    commands: Vec<String>,
}

#[derive(Debug)]
struct InstalledTool {
    package_id: String,
    version: String,
}

impl InstalledTool {
    fn target(&self, manager_id: &ManagerId) -> PackageTarget {
        let mut target = PackageTarget::new(manager_id.clone(), &self.package_id);
        target.scope = PackageScope::User;
        target.origin = Some(tool_origin(&self.package_id));
        target
    }

    fn info(self, manager_id: &ManagerId) -> PackageInfo {
        let mut info = PackageInfo::new(manager_id.clone(), &self.package_id, self.version);
        info.scope = PackageScope::User;
        info.origin = Some(tool_origin(&self.package_id));
        info
    }
}

fn parse_installed(value: &str) -> ManagerResult<Vec<InstalledTool>> {
    let document: ToolListDocument = serde_json::from_str(value).map_err(|error| {
        protocol(
            ".NET global tool listing JSON is malformed",
            &error.to_string(),
        )
    })?;
    if document.version != 1 {
        return Err(protocol(
            ".NET global tool listing schema is unsupported",
            &document.version.to_string(),
        ));
    }
    let mut identities = HashSet::new();
    let mut tools = document
        .data
        .into_iter()
        .map(|entry| {
            validate_package_id(&entry.package_id)?;
            validate_version(&entry.version)?;
            if entry.commands.is_empty()
                || entry.commands.iter().any(|command| {
                    command.is_empty()
                        || command.chars().any(char::is_whitespace)
                        || command.contains(['/', '\\'])
                })
            {
                return Err(protocol(
                    ".NET global tool command identity is malformed",
                    &entry.package_id,
                ));
            }
            let identity = entry.package_id.to_ascii_lowercase();
            if !identities.insert(identity) {
                return Err(protocol(
                    ".NET global tool listing contains a duplicate package ID",
                    &entry.package_id,
                ));
            }
            Ok(InstalledTool {
                package_id: entry.package_id,
                version: entry.version,
            })
        })
        .collect::<ManagerResult<Vec<_>>>()?;
    tools.sort_by_cached_key(|tool| tool.package_id.to_ascii_lowercase());
    Ok(tools)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackageSearchDocument {
    version: u64,
    #[serde(default)]
    problems: Vec<serde_json::Value>,
    search_result: Vec<PackageSearchSource>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackageSearchSource {
    source_name: String,
    packages: Vec<PackageVersion>,
}

#[derive(Debug, Deserialize)]
struct PackageVersion {
    id: String,
    version: String,
}

fn parse_latest_version(value: &str, package_id: &str) -> ManagerResult<String> {
    let document: PackageSearchDocument = serde_json::from_str(value).map_err(|error| {
        protocol(
            ".NET tool NuGet metadata JSON is malformed",
            &error.to_string(),
        )
    })?;
    if document.version != 2 {
        return Err(protocol(
            ".NET tool NuGet metadata schema is unsupported",
            &document.version.to_string(),
        ));
    }
    if !document.problems.is_empty() {
        return Err(protocol(
            ".NET tool NuGet metadata contains source problems",
            &serde_json::to_string(&document.problems).unwrap_or_default(),
        ));
    }
    let mut latest: Option<(Version, String)> = None;
    for source in document.search_result {
        if source.source_name.trim().is_empty() {
            return Err(protocol(
                ".NET tool NuGet source name is missing",
                package_id,
            ));
        }
        for package in source.packages {
            if !package.id.eq_ignore_ascii_case(package_id) {
                return Err(protocol(
                    ".NET tool NuGet metadata returned a different package ID",
                    &package.id,
                ));
            }
            validate_version(&package.version)?;
            let parsed = Version::parse(&package.version).map_err(|error| {
                protocol(
                    ".NET tool NuGet version is not semantic",
                    &format!("{}: {error}", package.version),
                )
            })?;
            if latest.as_ref().is_none_or(|(current, _)| parsed > *current) {
                latest = Some((parsed, package.version));
            }
        }
    }
    latest.map(|(_, version)| version).ok_or_else(|| {
        protocol(
            ".NET tool NuGet metadata did not contain the installed package",
            package_id,
        )
    })
}

fn parse_tool_search(value: &str, manager_id: &ManagerId) -> ManagerResult<Vec<PackageInfo>> {
    if value.trim() == "Could not find any results." {
        return Ok(Vec::new());
    }
    let mut packages = Vec::new();
    let mut identities = HashSet::new();
    for block in value.split("----------------").map(str::trim) {
        if block.is_empty() {
            continue;
        }
        let mut lines = block.lines();
        let package_id = lines
            .next()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .ok_or_else(|| protocol(".NET tool search package ID is missing", block))?;
        validate_package_id(package_id)?;
        let version = lines
            .next()
            .and_then(|line| line.trim().strip_prefix("Latest Version: "))
            .filter(|version| !version.is_empty())
            .ok_or_else(|| protocol(".NET tool search version is malformed", block))?;
        validate_version(version)?;
        let description = block
            .lines()
            .find_map(|line| line.trim().strip_prefix("Description: "))
            .filter(|description| !description.is_empty())
            .map(ToOwned::to_owned);
        let identity = package_id.to_ascii_lowercase();
        if !identities.insert(identity) {
            return Err(protocol(
                ".NET tool search contains a duplicate package ID",
                package_id,
            ));
        }
        let mut info = PackageInfo::new(manager_id.clone(), package_id, version);
        info.description = description;
        info.scope = PackageScope::User;
        info.origin = Some(tool_origin(package_id));
        packages.push(info);
    }
    packages.sort_by_cached_key(|package| package.name.to_ascii_lowercase());
    Ok(packages)
}

fn tool_origin(package_id: &str) -> PackageOrigin {
    PackageOrigin::new(".NET global tool").with_reference(format!("global:{package_id}"))
}

fn validate_origin(origin: &PackageOrigin, package_id: &str) -> ManagerResult<()> {
    let expected = format!("global:{package_id}");
    if origin.name == ".NET global tool"
        && origin
            .reference
            .as_deref()
            .is_some_and(|reference| reference.eq_ignore_ascii_case(&expected))
    {
        Ok(())
    } else {
        Err(protocol(".NET global tool origin is malformed", package_id))
    }
}

fn validate_package_id(value: &str) -> ManagerResult<()> {
    if value.is_empty()
        || value.starts_with(['-', '.'])
        || value.ends_with('.')
        || value.chars().any(char::is_whitespace)
        || value.chars().any(|character| {
            !(character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
        })
        || value.contains([';', '=', '/', '\\'])
    {
        return Err(protocol(".NET tool package ID is malformed", value));
    }
    Ok(())
}

fn validate_version(value: &str) -> ManagerResult<()> {
    if value.is_empty()
        || value.starts_with('-')
        || value.chars().any(char::is_whitespace)
        || value.contains(['/', '\\', ';', '='])
    {
        return Err(protocol(".NET tool version is malformed", value));
    }
    Ok(())
}

fn validate_search_term(value: &str) -> ManagerResult<()> {
    if value.starts_with('-') || value.contains(['\n', '\r', '\0']) {
        return Err(protocol(".NET tool search term is malformed", value));
    }
    Ok(())
}

fn ensure_supported_action(action: PackageAction) -> ManagerResult<()> {
    match action {
        PackageAction::Install | PackageAction::Update | PackageAction::Uninstall => Ok(()),
        _ => Err(ManagerError::new(
            ManagerErrorKind::Unsupported,
            ".NET global tool package action is unsupported",
        )),
    }
}

fn dotnet_command(path: &Path) -> CommandSpec {
    CommandSpec::new(path)
        .env("DOTNET_CLI_UI_LANGUAGE", "en-US")
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
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
    use super::*;

    #[test]
    fn installed_json_rejects_duplicate_case_insensitive_identity() {
        let error = parse_installed(
            r#"{"version":1,"data":[{"packageId":"Example.Tool","version":"1.0.0","commands":["example"]},{"packageId":"example.tool","version":"2.0.0","commands":["example2"]}]}"#,
        )
        .expect_err("duplicate IDs must be rejected");

        assert_eq!(error.kind(), ManagerErrorKind::Protocol);
    }

    #[test]
    fn exact_search_selects_highest_semantic_version_across_sources() {
        let latest = parse_latest_version(
            r#"{"version":2,"problems":[],"searchResult":[{"sourceName":"private","packages":[{"id":"Example.Tool","version":"2.9.0"}]},{"sourceName":"nuget.org","packages":[{"id":"example.tool","version":"2.10.0"},{"id":"example.tool","version":"1.0.0"}]}]}"#,
            "example.tool",
        )
        .expect("valid exact metadata");

        assert_eq!(latest, "2.10.0");
    }

    #[test]
    fn tool_search_preserves_package_identity_and_description() {
        let manager = DotnetToolManager::new();
        let packages = parse_tool_search(
            "----------------\nexample.tool\nLatest Version: 2.1.0\nAuthors: Example\nDownloads: 10\nVerified: False\nDescription: Example tool\nVersions:\n\t2.1.0 Downloads: 10\n",
            manager.descriptor().id(),
        )
        .expect("valid tool search");

        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].name, "example.tool");
        assert_eq!(packages[0].version, "2.1.0");
        assert_eq!(packages[0].description.as_deref(), Some("Example tool"));
        assert_eq!(packages[0].scope, PackageScope::User);
    }

    #[test]
    fn tool_search_accepts_the_cli_empty_result_contract() {
        let manager = DotnetToolManager::new();

        assert!(
            parse_tool_search("Could not find any results.\n", manager.descriptor().id())
                .expect("valid empty search")
                .is_empty()
        );
    }
}
