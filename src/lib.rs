//! Tier-0 project-model plugin for .NET workspaces.
//!
//! Emits the shared generic project-model JSON schema under `project-model:dotnet`
//! for solution and project roots.

use basalt_plugin_sdk::prelude::*;

basalt_plugin_meta! {
    name:         "dotnet-project-model",
    version:      "0.1.0",
    hook_flags:   CAP_PROJECT_MODEL,
    provides:     "project-model:dotnet",
    requires:     "",
    file_globs:   "",
    activates_on: "*.sln\n*.slnx\n*.csproj\n*.fsproj\n*.vbproj\n**/*.sln\n**/*.slnx\n**/*.csproj\n**/*.fsproj\n**/*.vbproj",
    activation_events: "",
}

extern "C" {
    fn basalt_read_file(path_ptr: i32, path_len: i32, out_ptr: i32, out_cap: i32) -> i32;
}

const FILE_BUF_SIZE: usize = 4 * 1024 * 1024;

static mut FILE_BUF: [u8; FILE_BUF_SIZE] = [0u8; FILE_BUF_SIZE];

#[derive(Clone)]
struct ProjectInfo {
    id: String,
    name: String,
    kind: String,
    subtype: Option<String>,
    capabilities: Vec<String>,
    manifest_path: String,
    source_roots: Vec<String>,
    test_roots: Vec<String>,
    build_roots: Vec<String>,
    target_name: String,
    target_kind: String,
    target_subtype: Option<String>,
    target_capabilities: Vec<String>,
    owner_prefix: String,
}

#[basalt_plugin]
fn build_project_model(root: &str) -> Vec<u8> {
    if is_solution_path(root) {
        return build_solution_model(root).into_bytes();
    }
    if is_project_path(root) {
        return build_project_file_model(root).into_bytes();
    }
    Vec::new()
}

fn build_solution_model(solution_path: &str) -> String {
    let solution = read_host_file(solution_path).unwrap_or_default();
    if solution.is_empty() {
        return String::new();
    }

    let solution_dir = parent_dir(solution_path);
    let display_name = file_stem(solution_path).to_string();
    let mut projects = vec![ProjectInfo {
        id: sanitize_id(&display_name),
        name: display_name.clone(),
        kind: "solution".to_string(),
        subtype: Some("dotnet-solution".to_string()),
        capabilities: Vec::new(),
        manifest_path: solution_path.to_string(),
        source_roots: Vec::new(),
        test_roots: Vec::new(),
        build_roots: Vec::new(),
        target_name: display_name.clone(),
        target_kind: "solution".to_string(),
        target_subtype: Some("dotnet-solution".to_string()),
        target_capabilities: Vec::new(),
        owner_prefix: String::new(),
    }];
    for (name, rel_path) in parse_solution_projects(&solution) {
        let normalized_rel = normalize_rel_path(&rel_path);
        if !is_project_path(&normalized_rel) {
            continue;
        }
        let manifest_path = join_path(&solution_dir, &normalized_rel);
        if let Some(info) = parse_project_info(&manifest_path, Some(name), Some(&normalized_rel)) {
            projects.push(info);
        }
    }

    emit_model("dotnet", &solution_dir, &display_name, solution_path, &projects)
}

fn build_project_file_model(project_path: &str) -> String {
    let display_name = file_stem(project_path).to_string();
    let Some(project) = parse_project_info(project_path, Some(display_name.clone()), None) else {
        return String::new();
    };
    let root = parent_dir(project_path);
    emit_model("dotnet", &root, &display_name, project_path, &[project])
}

fn parse_project_info(
    manifest_path: &str,
    fallback_name: Option<String>,
    owner_prefix: Option<&str>,
) -> Option<ProjectInfo> {
    let manifest = read_host_file(manifest_path)?;
    if manifest.is_empty() {
        return None;
    }

    let project_name = parse_xml_text(&manifest, "AssemblyName")
        .or_else(|| parse_xml_text(&manifest, "RootNamespace"))
        .or(fallback_name)
        .unwrap_or_else(|| file_stem(manifest_path).to_string());

    let is_test = parse_bool_tag(&manifest, "IsTestProject")
        || manifest.contains("Microsoft.NET.Test.Sdk")
        || manifest.contains("xunit")
        || manifest.contains("NUnit")
        || manifest.contains("MSTest");

    let sdk = parse_project_sdk(&manifest).unwrap_or_default();
    let output_type = parse_xml_text(&manifest, "OutputType").unwrap_or_default();
    let uses_aspnet = sdk.contains("Microsoft.NET.Sdk.Web")
        || manifest.contains("Microsoft.AspNetCore.App")
        || manifest.contains("Microsoft.AspNetCore.");
    let uses_worker = sdk.contains("Microsoft.NET.Sdk.Worker")
        || manifest.contains("Microsoft.Extensions.Hosting")
        || manifest.contains("WorkerService");
    let uses_blazor = manifest.contains("Microsoft.AspNetCore.Components.WebAssembly")
        || manifest.contains("blazor");
    let exposes_openapi = manifest.contains("Swashbuckle")
        || manifest.contains("Microsoft.AspNetCore.OpenApi")
        || manifest.contains("OpenApi");
    let is_aspire = manifest.contains("Aspire.Hosting.AppHost")
        || parse_bool_tag(&manifest, "IsAspireHost")
        || parse_bool_tag(&manifest, "UseAspire");

    let (kind, subtype, mut capabilities) = if is_test {
        (
            "test".to_string(),
            Some("test-project".to_string()),
            vec!["is-test-only".to_string(), "is-executable".to_string()],
        )
    } else if is_aspire {
        (
            "app".to_string(),
            Some("aspire-apphost".to_string()),
            vec!["is-executable".to_string(), "orchestrates-services".to_string()],
        )
    } else if uses_blazor {
        (
            "app".to_string(),
            Some("web-app".to_string()),
            vec!["has-ui".to_string(), "serves-http".to_string(), "is-executable".to_string()],
        )
    } else if uses_worker {
        (
            "app".to_string(),
            Some("worker".to_string()),
            vec!["is-executable".to_string(), "runs-background-jobs".to_string()],
        )
    } else if uses_aspnet {
        (
            "app".to_string(),
            Some(if exposes_openapi { "web-api" } else { "web-app" }.to_string()),
            vec!["serves-http".to_string(), "is-executable".to_string()],
        )
    } else if matches!(output_type.as_str(), "Exe" | "WinExe") {
        (
            "app".to_string(),
            Some("console-app".to_string()),
            vec!["is-executable".to_string()],
        )
    } else {
        (
            "library".to_string(),
            Some("class-library".to_string()),
            vec!["is-library".to_string()],
        )
    };
    if uses_aspnet && !capabilities.iter().any(|c| c == "exposes-api") && exposes_openapi {
        capabilities.push("exposes-api".to_string());
    }
    if uses_aspnet && subtype.as_deref() == Some("web-api") && !capabilities.iter().any(|c| c == "exposes-api") {
        capabilities.push("exposes-api".to_string());
    }

    let project_dir = parent_dir(manifest_path);
    let owner_prefix = owner_prefix
        .map(normalize_rel_path)
        .unwrap_or_default();
    let source_roots = if owner_prefix.is_empty() {
        vec![".".to_string()]
    } else {
        vec![ensure_trailing_slash(&owner_prefix)]
    };
    let test_roots = if is_test { source_roots.clone() } else { Vec::new() };
    let build_roots = if owner_prefix.is_empty() {
        vec!["bin/".to_string(), "obj/".to_string()]
    } else {
        vec![
            format!("{}bin/", ensure_trailing_slash(&owner_prefix)),
            format!("{}obj/", ensure_trailing_slash(&owner_prefix)),
        ]
    };

    Some(ProjectInfo {
        id: sanitize_id(&project_name),
        name: project_name.clone(),
        kind: kind.clone(),
        subtype: subtype.clone(),
        capabilities: capabilities.clone(),
        manifest_path: manifest_path.to_string(),
        source_roots,
        test_roots,
        build_roots,
        target_name: project_name,
        target_kind: kind,
        target_subtype: subtype,
        target_capabilities: capabilities,
        owner_prefix: if owner_prefix.is_empty() {
            relative_name_from_dir(&project_dir)
        } else {
            owner_prefix
        },
    })
}

fn emit_model(
    ecosystem: &str,
    root: &str,
    display_name: &str,
    primary_manifest_path: &str,
    projects: &[ProjectInfo],
) -> String {
    let projects_json = projects
        .iter()
        .map(project_json)
        .collect::<Vec<_>>()
        .join(",");
    let owners_json = projects
        .iter()
        .flat_map(project_owner_json)
        .collect::<Vec<_>>()
        .join(",");

    format!(
        concat!(
            "{{",
            "\"schema_version\":1,",
            "\"ecosystem\":\"{}\",",
            "\"root\":\"{}\",",
            "\"display_name\":\"{}\",",
            "\"primary_manifest_path\":\"{}\",",
            "\"projects\":[{}],",
            "\"owners\":[{}]",
            "}}"
        ),
        escape_json(ecosystem),
        escape_json(root),
        escape_json(display_name),
        escape_json(primary_manifest_path),
        projects_json,
        owners_json,
    )
}

fn project_json(project: &ProjectInfo) -> String {
    format!(
        concat!(
            "{{",
            "\"id\":\"{}\",",
            "\"name\":\"{}\",",
            "\"kind\":\"{}\",",
            "\"subtype\":{},",
            "\"capabilities\":[{}],",
            "\"manifest_path\":\"{}\",",
            "\"source_roots\":[{}],",
            "\"test_roots\":[{}],",
            "\"build_roots\":[{}],",
            "\"targets\":[{{",
                "\"id\":\"{}\",",
                "\"name\":\"{}\",",
                "\"kind\":\"{}\",",
                "\"subtype\":{},",
                "\"capabilities\":[{}],",
                "\"language\":\"{}\",",
                "\"source_roots\":[{}],",
                "\"test_roots\":[{}],",
                "\"build_roots\":[{}]",
            "}}]",
            "}}"
        ),
        escape_json(&project.id),
        escape_json(&project.name),
        escape_json(&project.kind),
        quote_opt(&project.subtype),
        quote_list(&project.capabilities),
        escape_json(&project.manifest_path),
        quote_list(&project.source_roots),
        quote_list(&project.test_roots),
        quote_list(&project.build_roots),
        escape_json(&project.id),
        escape_json(&project.target_name),
        escape_json(&project.target_kind),
        quote_opt(&project.target_subtype),
        quote_list(&project.target_capabilities),
        escape_json(project_language(&project.manifest_path)),
        quote_list(&project.source_roots),
        quote_list(&project.test_roots),
        quote_list(&project.build_roots),
    )
}

fn project_owner_json(project: &ProjectInfo) -> Vec<String> {
    let mut out = Vec::new();
    if project.kind == "solution" {
        out.push(owner_json(
            file_name(&project.manifest_path),
            "exact",
            &project.name,
            &project.id,
            "manifest",
        ));
        return out;
    }
    let manifest_rel = normalize_rel_path(&project.owner_prefix);
    if !manifest_rel.is_empty() && manifest_rel != "." {
        out.push(owner_json(
            &format!("{manifest_rel}/{}", file_name(&project.manifest_path)),
            "exact",
            &project.name,
            &project.id,
            "manifest",
        ));
        out.push(owner_json(
            &ensure_trailing_slash(&manifest_rel),
            "prefix",
            &project.name,
            &project.id,
            if project.kind == "test" { "test" } else { "source" },
        ));
    } else {
        out.push(owner_json(
            file_name(&project.manifest_path),
            "exact",
            &project.name,
            &project.id,
            "manifest",
        ));
    }
    out
}

fn parse_project_sdk(contents: &str) -> Option<String> {
    if let Some(start) = contents.find("<Project Sdk=\"") {
        let value_start = start + "<Project Sdk=\"".len();
        let value_end = contents[value_start..].find('"')? + value_start;
        return Some(contents[value_start..value_end].to_string());
    }
    parse_xml_text(contents, "Sdk")
}

fn owner_json(path: &str, match_kind: &str, label: &str, project_id: &str, kind: &str) -> String {
    format!(
        "{{\"path\":\"{}\",\"match_kind\":\"{}\",\"label\":\"{}\",\"project_id\":\"{}\",\"target_id\":\"{}\",\"kind\":\"{}\"}}",
        escape_json(path),
        escape_json(match_kind),
        escape_json(label),
        escape_json(project_id),
        escape_json(project_id),
        escape_json(kind),
    )
}

fn parse_solution_projects(contents: &str) -> Vec<(String, String)> {
    if contents.contains("<Solution") {
        return parse_slnx_projects(contents);
    }

    let mut out = Vec::new();
    for line in contents.lines() {
        let line = line.trim();
        if !line.starts_with("Project(") {
            continue;
        }
        let Some((_, rhs)) = line.split_once('=') else { continue };
        let mut parts = rhs.split(',').map(|part| part.trim().trim_matches('"'));
        let Some(name) = parts.next() else { continue };
        let Some(path) = parts.next() else { continue };
        if path.is_empty() || name.is_empty() {
            continue;
        }
        out.push((name.to_string(), path.to_string()));
    }
    out
}

fn parse_slnx_projects(contents: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in contents.lines() {
        let line = line.trim();
        if !line.starts_with("<Project ") {
            continue;
        }
        let Some(path) = parse_attr(line, "Path") else { continue };
        let normalized = normalize_rel_path(&path);
        if !is_project_path(&normalized) {
            continue;
        }
        let name = file_stem(&normalized).to_string();
        out.push((name, normalized));
    }
    out
}

fn parse_attr(line: &str, attr: &str) -> Option<String> {
    let needle = format!(r#"{attr}=""#);
    let start = line.find(&needle)? + needle.len();
    let end = line[start..].find('"')? + start;
    let value = line[start..end].trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn parse_bool_tag(contents: &str, tag: &str) -> bool {
    parse_xml_text(contents, tag)
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn parse_xml_text(contents: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = contents.find(&open)? + open.len();
    let end = contents[start..].find(&close)? + start;
    let value = contents[start..end].trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn read_host_file(path: &str) -> Option<String> {
    let buf: &mut [u8; FILE_BUF_SIZE] = unsafe { &mut *core::ptr::addr_of_mut!(FILE_BUF) };
    let n = unsafe {
        basalt_read_file(
            path.as_ptr() as i32,
            path.len() as i32,
            buf.as_mut_ptr() as i32,
            FILE_BUF_SIZE as i32,
        )
    };
    if n <= 0 {
        return None;
    }
    let bytes = &buf[..n as usize];
    Some(String::from_utf8_lossy(bytes).into_owned())
}

fn is_solution_path(path: &str) -> bool {
    path.ends_with(".sln") || path.ends_with(".slnx")
}

fn is_project_path(path: &str) -> bool {
    path.ends_with(".csproj") || path.ends_with(".fsproj") || path.ends_with(".vbproj")
}

fn project_language(path: &str) -> &'static str {
    if path.ends_with(".fsproj") {
        "fsharp"
    } else if path.ends_with(".vbproj") {
        "vb"
    } else {
        "csharp"
    }
}

fn normalize_rel_path(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches("./").trim_matches('/').to_string()
}

fn join_path(root: &str, suffix: &str) -> String {
    if root.is_empty() {
        return suffix.to_string();
    }
    if root.ends_with('/') {
        format!("{root}{suffix}")
    } else {
        format!("{root}/{suffix}")
    }
}

fn parent_dir(path: &str) -> String {
    path.rsplit_once(['/', '\\'])
        .map(|(parent, _)| parent.to_string())
        .unwrap_or_default()
}

fn file_stem(path: &str) -> &str {
    let name = file_name(path);
    name.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(name)
}

fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn relative_name_from_dir(path: &str) -> String {
    let name = file_name(path);
    if name.is_empty() {
        ".".to_string()
    } else {
        ".".to_string()
    }
}

fn ensure_trailing_slash(path: &str) -> String {
    if path.is_empty() || path.ends_with('/') {
        path.to_string()
    } else {
        format!("{path}/")
    }
}

fn quote_list(items: &[String]) -> String {
    items.iter()
        .map(|item| format!("\"{}\"", escape_json(item)))
        .collect::<Vec<_>>()
        .join(",")
}

fn quote_opt(value: &Option<String>) -> String {
    match value {
        Some(value) => format!("\"{}\"", escape_json(value)),
        None => "null".to_string(),
    }
}

fn sanitize_id(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "dotnet-project".to_string();
    }
    trimmed
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn escape_json(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 16);
    for ch in input.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < ' ' => {
                let n = c as u32;
                out.push_str("\\u");
                out.push(char::from(b"0123456789ABCDEF"[((n >> 12) & 0xf) as usize]));
                out.push(char::from(b"0123456789ABCDEF"[((n >> 8) & 0xf) as usize]));
                out.push(char::from(b"0123456789ABCDEF"[((n >> 4) & 0xf) as usize]));
                out.push(char::from(b"0123456789ABCDEF"[(n & 0xf) as usize]));
            }
            c => out.push(c),
        }
    }
    out
}
