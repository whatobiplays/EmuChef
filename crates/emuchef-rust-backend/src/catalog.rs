//! Authored-root catalog context used by validation.
//!
//! Validation scans these authored-root-relative top-level patterns:
//! `apps/*.y*ml`, `recipes/*.y*ml`,
//! `device_profiles/*.y*ml`, and `device_plans/*.y*ml`. The catalog models only
//! recipe metadata needed for recipe dependency diagnostics.
//! It does not build planner data structures, execution plans, or executor
//! state.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::model::Recipe;
use crate::yaml;

pub const AUTHORED_CATALOG_GLOBS: &[&str] = &[
    "apps/*.y*ml",
    "recipes/*.y*ml",
    "device_profiles/*.y*ml",
    "device_plans/*.y*ml",
];

const CATALOG_DIRECTORIES: &[&str] = &["apps", "recipes", "device_profiles", "device_plans"];

#[derive(Clone, Debug, Default)]
struct ValidationCatalog {
    recipes: HashMap<String, Recipe>,
    recipe_files: HashMap<String, Vec<PathBuf>>,
}

pub fn normalize_authored_root(authored_root: Option<&str>, recipe_path: &Path) -> Option<PathBuf> {
    if let Some(root) = authored_root {
        let root_path = resolved_path(root);
        let authored_candidate = root_path.join("authored");
        if root_path.file_name().and_then(|name| name.to_str()) == Some("authored")
            && root_path.join("recipes").is_dir()
        {
            return Some(root_path);
        }
        if authored_candidate.is_dir() && authored_candidate.join("recipes").is_dir() {
            return Some(authored_candidate);
        }
        return Some(root_path);
    }

    infer_authored_root(recipe_path)
}

pub fn validate_recipe_with_catalog(
    file: &str,
    recipe: &Recipe,
    recipe_path: &Path,
    authored_root: &Path,
) -> Vec<Value> {
    let catalog = ValidationCatalog::collect(authored_root);
    catalog.validate_recipe(file, recipe, recipe_path, authored_root)
}

fn infer_authored_root(recipe_path: &Path) -> Option<PathBuf> {
    let target_path = PathBuf::from(yaml::resolved_path_string(recipe_path));
    for parent in target_path.ancestors().skip(1) {
        if parent.file_name().and_then(|name| name.to_str()) == Some("authored")
            && parent.join("recipes").is_dir()
        {
            return Some(parent.to_path_buf());
        }
    }
    None
}

fn resolved_path(path: impl AsRef<Path>) -> PathBuf {
    PathBuf::from(yaml::resolved_path_string(path.as_ref()))
}

impl ValidationCatalog {
    fn collect(authored_root: &Path) -> Self {
        let mut catalog = Self::default();

        for directory_name in CATALOG_DIRECTORIES {
            for path in top_level_yaml_files(&authored_root.join(directory_name)) {
                if *directory_name == "recipes" {
                    catalog.collect_recipe(path);
                }
            }
        }

        catalog
    }

    fn collect_recipe(&mut self, path: PathBuf) {
        let Ok(raw) = yaml::load_yaml_mapping(&path) else {
            return;
        };
        let Ok(recipe) = yaml::parse_recipe_mapping(&raw, &path) else {
            return;
        };
        self.recipe_files
            .entry(recipe.id.clone())
            .or_default()
            .push(resolved_path(&path));
        if self.recipes.contains_key(&recipe.id) {
            return;
        }
        self.recipes.insert(recipe.id.clone(), recipe);
    }

    fn validate_recipe(
        &self,
        file: &str,
        recipe: &Recipe,
        recipe_path: &Path,
        authored_root: &Path,
    ) -> Vec<Value> {
        let target_path = resolved_path(recipe_path);
        let resolved_authored_root = resolved_path(authored_root);
        let target_in_catalog_root = target_path.starts_with(&resolved_authored_root);
        let mut recipes = self.recipes.clone();

        let replaced_recipe_id = first_recipe_id_for_path(&self.recipe_files, &target_path);
        if let Some(existing_id) = &replaced_recipe_id {
            recipes.remove(existing_id);
        }

        let mut errors = Vec::new();
        let existing_path = (replaced_recipe_id.as_deref() != Some(recipe.id.as_str()))
            .then(|| {
                self.recipe_files
                    .get(&recipe.id)
                    .and_then(|paths| paths.first())
            })
            .flatten();
        if target_in_catalog_root && existing_path.is_some_and(|path| path != &target_path) {
            errors.push(diagnostic(
                "error",
                "recipe_id_conflict",
                &format!("Duplicate recipe id {}.", single_quote(&recipe.id)),
                file,
                Some("recipe"),
                Some(&recipe.id),
                Some("id"),
            ));
        }

        recipes.insert(recipe.id.clone(), recipe.clone());

        for (index, dependency_ref) in recipe.recipe_dependencies.iter().enumerate() {
            if !recipes.contains_key(dependency_ref) {
                errors.push(diagnostic(
                    "error",
                    "recipe_not_found",
                    &format!(
                        "Recipe dependency {} was not found.",
                        single_quote(dependency_ref)
                    ),
                    file,
                    Some("recipe"),
                    Some(&recipe.id),
                    Some(&format!("recipe_dependencies[{index}]")),
                ));
            }
        }

        if dependency_cycle_reachable(&recipes, &recipe.id) {
            errors.push(diagnostic(
                "error",
                "dependency_cycle",
                &format!(
                    "Recipe dependency cycle detected in recipe {}.",
                    single_quote(&recipe.id)
                ),
                file,
                Some("recipe"),
                Some(&recipe.id),
                Some("recipe_dependencies"),
            ));
        }

        errors
    }
}

fn top_level_yaml_files(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_yaml_extension(path))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn is_yaml_extension(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("yaml" | "yml")
    )
}

fn first_recipe_id_for_path(
    recipe_files: &HashMap<String, Vec<PathBuf>>,
    target_path: &Path,
) -> Option<String> {
    recipe_files.iter().find_map(|(recipe_id, recipe_path)| {
        recipe_path
            .first()
            .is_some_and(|path| path == target_path)
            .then(|| recipe_id.clone())
    })
}

fn dependency_cycle_reachable(recipes: &HashMap<String, Recipe>, selected_recipe_id: &str) -> bool {
    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    visit_recipe_dependency(recipes, selected_recipe_id, &mut visiting, &mut visited)
}

fn visit_recipe_dependency(
    recipes: &HashMap<String, Recipe>,
    recipe_id: &str,
    visiting: &mut HashSet<String>,
    visited: &mut HashSet<String>,
) -> bool {
    if visiting.contains(recipe_id) {
        return true;
    }
    if visited.contains(recipe_id) {
        return false;
    }
    let Some(recipe) = recipes.get(recipe_id) else {
        return false;
    };

    visiting.insert(recipe_id.to_string());
    for dependency_ref in &recipe.recipe_dependencies {
        if visit_recipe_dependency(recipes, dependency_ref, visiting, visited) {
            return true;
        }
    }
    visiting.remove(recipe_id);
    visited.insert(recipe_id.to_string());
    false
}

fn diagnostic(
    severity: &str,
    code: &str,
    message: &str,
    file: &str,
    object_kind: Option<&str>,
    object_id: Option<&str>,
    field: Option<&str>,
) -> Value {
    json!({
        "severity": severity,
        "code": code,
        "message": message,
        "file": file,
        "objectKind": object_kind,
        "objectId": object_id,
        "field": field,
    })
}

fn single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}
