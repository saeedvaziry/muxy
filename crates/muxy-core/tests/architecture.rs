use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

type Graph = BTreeMap<String, BTreeSet<String>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VisitState {
    Visiting,
    Visited,
}

#[test]
fn workspace_dependencies_follow_architecture_policy() -> Result<(), String> {
    let metadata = load_metadata()?;
    let (actual, dev) = workspace_graph(&metadata)?;
    let allowed = policy_graph(&metadata, "allowed-dependencies")?
        .ok_or_else(|| "workspace architecture policy is missing".to_owned())?;
    let allowed_dev = policy_graph(&metadata, "allowed-dev-dependencies")?.unwrap_or_default();
    let mut violations = validate_architecture(&actual, &allowed);
    violations.extend(validate_dev_dependencies(&dev, &allowed, &allowed_dev));

    assert!(
        violations.is_empty(),
        "architecture violations:\n{}",
        violations.join("\n")
    );
    Ok(())
}

#[test]
fn rejects_a_forbidden_dependency() {
    let actual = fixture(&[("app", &["server"]), ("server", &[])]);
    let allowed = fixture(&[("app", &[]), ("server", &[])]);
    let violations = validate_architecture(&actual, &allowed);

    assert!(
        violations
            .iter()
            .any(|violation| violation == "app may not depend on server"),
        "{violations:#?}"
    );
}

#[test]
fn rejects_a_dependency_cycle() {
    let cycle = fixture(&[("first", &["second"]), ("second", &["first"])]);
    let violations = validate_architecture(&cycle, &cycle);

    assert!(
        violations
            .iter()
            .any(|violation| violation.contains("dependency graph contains a cycle")),
        "{violations:#?}"
    );
}

#[test]
fn requires_a_new_workspace_crate_to_declare_its_boundary() {
    let actual = fixture(&[("managed", &[]), ("new-crate", &[])]);
    let allowed = fixture(&[("managed", &[])]);
    let violations = validate_architecture(&actual, &allowed);

    assert!(
        violations
            .iter()
            .any(|violation| violation == "new-crate must declare an architecture policy"),
        "{violations:#?}"
    );
}

#[test]
fn dev_dependencies_use_the_test_only_table_on_top_of_production() {
    let dev = fixture(&[("client", &["server", "protocol"])]);
    let allowed = fixture(&[
        ("client", &["protocol"]),
        ("server", &[]),
        ("protocol", &[]),
    ]);
    let allowed_dev = fixture(&[("client", &["server"])]);

    assert!(validate_dev_dependencies(&dev, &allowed, &allowed_dev).is_empty());

    let violations = validate_dev_dependencies(&dev, &allowed, &Graph::new());
    assert_eq!(
        violations,
        vec!["client may not have a test-only dependency on server".to_owned()]
    );

    let unknown = fixture(&[("client", &["ghost"]), ("ghost", &[])]);
    let violations = validate_dev_dependencies(&dev, &allowed, &unknown);
    assert!(
        violations
            .iter()
            .any(|violation| violation == "test-only policy for client names unknown crate ghost"),
        "{violations:#?}"
    );
    assert!(
        violations
            .iter()
            .any(|violation| violation == "test-only policy names unknown crate ghost"),
        "{violations:#?}"
    );
}

fn load_metadata() -> Result<Value, String> {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version=1", "--no-deps"])
        .current_dir(workspace_root)
        .output()
        .map_err(|error| format!("failed to run cargo metadata: {error}"))?;

    if !output.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("cargo metadata returned invalid JSON: {error}"))
}

fn workspace_graph(metadata: &Value) -> Result<(Graph, Graph), String> {
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| "cargo metadata has no packages array".to_owned())?;
    let member_ids = metadata
        .get("workspace_members")
        .and_then(Value::as_array)
        .ok_or_else(|| "cargo metadata has no workspace_members array".to_owned())?
        .iter()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    let workspace_names = packages
        .iter()
        .filter(|package| {
            package
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| member_ids.contains(id))
        })
        .filter_map(|package| package.get("name").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    let mut graph = Graph::new();
    let mut dev_graph = Graph::new();

    for package in packages {
        let id = string_field(package, "id")?;
        if !member_ids.contains(id) {
            continue;
        }

        let name = string_field(package, "name")?;
        let dependencies = package
            .get("dependencies")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{name} has no dependencies array"))?;
        let mut workspace_dependencies = BTreeSet::new();
        let mut dev_dependencies = BTreeSet::new();
        for dependency in dependencies {
            let dependency_name = string_field(dependency, "name")?;
            if !workspace_names.contains(dependency_name) {
                continue;
            }
            if dependency.get("kind").and_then(Value::as_str) == Some("dev") {
                dev_dependencies.insert(dependency_name.to_owned());
            } else {
                workspace_dependencies.insert(dependency_name.to_owned());
            }
        }
        graph.insert(name.to_owned(), workspace_dependencies);
        if !dev_dependencies.is_empty() {
            dev_graph.insert(name.to_owned(), dev_dependencies);
        }
    }

    Ok((graph, dev_graph))
}

fn policy_graph(metadata: &Value, table: &str) -> Result<Option<Graph>, String> {
    let Some(policy) = metadata.pointer(&format!("/metadata/architecture/{table}")) else {
        return Ok(None);
    };
    let policy = policy
        .as_object()
        .ok_or_else(|| format!("architecture table {table} is not a table"))?;
    let mut graph = Graph::new();

    for (name, dependencies) in policy {
        let dependencies = dependencies
            .as_array()
            .ok_or_else(|| format!("architecture policy for {name} is not an array"))?
            .iter()
            .map(|dependency| {
                dependency
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| format!("architecture dependency for {name} is not a string"))
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        graph.insert(name.clone(), dependencies);
    }

    Ok(Some(graph))
}

fn string_field<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("cargo metadata field {field} is missing or not a string"))
}

fn validate_architecture(actual: &Graph, allowed: &Graph) -> Vec<String> {
    let mut violations = Vec::new();

    for name in actual.keys() {
        if !allowed.contains_key(name) {
            violations.push(format!("{name} must declare an architecture policy"));
        }
    }
    for name in allowed.keys() {
        if !actual.contains_key(name) {
            violations.push(format!("architecture policy names missing crate {name}"));
        }
    }
    for (name, dependencies) in allowed {
        for dependency in dependencies {
            if !allowed.contains_key(dependency) {
                violations.push(format!(
                    "architecture policy for {name} names unknown crate {dependency}"
                ));
            }
        }
    }
    for (name, dependencies) in actual {
        let Some(permitted) = allowed.get(name) else {
            continue;
        };
        for dependency in dependencies {
            if !permitted.contains(dependency) {
                violations.push(format!("{name} may not depend on {dependency}"));
            }
        }
    }
    if let Some(cycle) = find_cycle(allowed) {
        violations.push(format!(
            "allowed dependency graph contains a cycle: {}",
            cycle.join(" -> ")
        ));
    }
    if let Some(cycle) = find_cycle(actual) {
        violations.push(format!(
            "workspace dependency graph contains a cycle: {}",
            cycle.join(" -> ")
        ));
    }

    violations
}

fn validate_dev_dependencies(dev: &Graph, allowed: &Graph, allowed_dev: &Graph) -> Vec<String> {
    let mut violations = Vec::new();

    for (name, dependencies) in allowed_dev {
        if !allowed.contains_key(name) {
            violations.push(format!("test-only policy names unknown crate {name}"));
        }
        for dependency in dependencies {
            if !allowed.contains_key(dependency) {
                violations.push(format!(
                    "test-only policy for {name} names unknown crate {dependency}"
                ));
            }
        }
    }
    for (name, dependencies) in dev {
        for dependency in dependencies {
            let production = allowed
                .get(name)
                .is_some_and(|set| set.contains(dependency));
            let test_only = allowed_dev
                .get(name)
                .is_some_and(|set| set.contains(dependency));
            if !production && !test_only {
                violations.push(format!(
                    "{name} may not have a test-only dependency on {dependency}"
                ));
            }
        }
    }

    violations
}

fn find_cycle(graph: &Graph) -> Option<Vec<String>> {
    let mut states = BTreeMap::new();
    let mut stack = Vec::new();

    for node in graph.keys() {
        if !states.contains_key(node)
            && let Some(cycle) = visit(node, graph, &mut states, &mut stack)
        {
            return Some(cycle);
        }
    }

    None
}

fn visit(
    node: &str,
    graph: &Graph,
    states: &mut BTreeMap<String, VisitState>,
    stack: &mut Vec<String>,
) -> Option<Vec<String>> {
    states.insert(node.to_owned(), VisitState::Visiting);
    stack.push(node.to_owned());

    if let Some(dependencies) = graph.get(node) {
        for dependency in dependencies {
            match states.get(dependency).copied() {
                Some(VisitState::Visiting) => {
                    let start = stack
                        .iter()
                        .position(|candidate| candidate == dependency)
                        .unwrap_or_default();
                    let mut cycle = stack[start..].to_vec();
                    cycle.push(dependency.clone());
                    return Some(cycle);
                }
                Some(VisitState::Visited) => {}
                None => {
                    if let Some(cycle) = visit(dependency, graph, states, stack) {
                        return Some(cycle);
                    }
                }
            }
        }
    }

    stack.pop();
    states.insert(node.to_owned(), VisitState::Visited);
    None
}

fn fixture(entries: &[(&str, &[&str])]) -> Graph {
    entries
        .iter()
        .map(|(name, dependencies)| {
            (
                (*name).to_owned(),
                dependencies
                    .iter()
                    .map(|dependency| (*dependency).to_owned())
                    .collect(),
            )
        })
        .collect()
}

#[test]
fn application_shortcuts_cannot_bypass_the_shared_registration_interface()
-> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut pending = vec![
        root.join("crates/muxy-app/src"),
        root.join("crates/muxy-ui/src"),
    ];
    while let Some(path) = pending.pop() {
        if path.is_dir() {
            pending.extend(
                std::fs::read_dir(path)?
                    .map(|entry| entry.map(|entry| entry.path()))
                    .collect::<std::io::Result<Vec<_>>>()?,
            );
            continue;
        }
        if path.extension().is_none_or(|extension| extension != "rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path)?;
        let production = text.split("#[cfg(test)]").next().unwrap_or("");
        if !path.ends_with("muxy-ui/src/shortcuts.rs") {
            assert!(
                !production.contains("KeyBinding::new("),
                "{} bypasses the shortcut registry",
                path.display()
            );
        }
        if path.ends_with("views/terminal/pane.rs") {
            assert!(!production.contains("keystroke.key"));
            assert!(!production.contains("keystroke.modifiers"));
            for line in production
                .lines()
                .filter(|line| line.contains(".on_key_down("))
            {
                assert_eq!(
                    line.trim(),
                    ".on_key_down(cx.listener(Self::terminal_key_down))"
                );
            }
            let handler = production
                .split("fn terminal_key_down(")
                .nth(1)
                .ok_or("terminal input handler")?;
            let handler = handler
                .split("\n}\n")
                .next()
                .ok_or("terminal input implementation")?;
            assert!(handler.contains("input::encode(&event.keystroke, grid.modes)"));
            assert!(!handler.contains("close_find"));
        } else {
            assert!(
                !production.contains(".on_key_down("),
                "{} handles shortcuts outside the registry",
                path.display()
            );
        }
    }
    Ok(())
}
