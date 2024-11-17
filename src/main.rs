use anyhow::{anyhow, Context};
use clap::{arg, command, Parser};
use dotenvy::dotenv;
use expanduser::expanduser;
use log::error;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::{
    env::{self},
    path::{Path, PathBuf},
};
use thiserror::Error;
use walkdir::WalkDir;

use std::process::{Command, Stdio};

use anyhow::Result;

#[derive(Error, Debug)]
enum YamlError {
    #[error("Expected 'tags' to be an array, but found a different type")]
    InvalidTagsType,
    #[error("Failed to parse YAML front matter: {0}")]
    ParseError(#[from] yaml_rust::ScanError),
    #[error("Failed to load file: {0}")]
    LoadError(#[from] std::io::Error),
}

fn read_first_section(path: &Path) -> Result<String, YamlError> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);

    let mut in_section = false;
    let mut current_section = String::new();

    for line in reader.lines() {
        let line = line?;

        if line.trim() == "---" {
            if in_section {
                // End of the section, append "---" and return the result
                current_section.push_str("---\n");
                return Ok(current_section);
            } else {
                // Start a new section, append "---"
                in_section = true;
                current_section.push_str("---\n");
            }
        } else if in_section {
            // Buffer lines in the current section
            current_section.push_str(&line);
            current_section.push('\n');
        }
    }

    // If we reach the end of the file but no closing `---` is found, return the buffered content.
    if in_section {
        return Ok(current_section);
    }

    // If no section is found, return an empty string.
    Ok(String::new())
}

type Tags = Vec<String>;

fn load_tags(path: &Path) -> Result<Tags, YamlError> {
    let content = read_first_section(path)?;
    let items = frontmatter::parse(&content).map_err(YamlError::ParseError)?;
    let make_tag = |s: &str| -> Option<String> {
        let s = s.trim();
        if !s.is_empty() {
            Some(String::from(s))
        } else {
            None
        }
    };
    match items {
        None => Ok(Vec::new()),
        Some(yaml) => match yaml["tags"].as_vec() {
            Some(tags) => Ok(tags
                .iter()
                .filter_map(|tag| tag.as_str().and_then(make_tag))
                .collect()),
            None => Err(YamlError::InvalidTagsType),
        },
    }
}

/// Obsidianタグを収集するイテレータを返す関数
///
/// # Arguments
/// * `directory` - タグを検索するディレクトリパス
///
/// # Returns
/// タグの文字列イテレータ
fn collect_obsidian_tags(
    directory: &str,
) -> anyhow::Result<impl Iterator<Item = Result<String, std::io::Error>>> {
    let command = Command::new("rg")
        .arg("--pcre2")
        .arg("-o")
        .arg(r#"(?<=\s)#[^\s\#\|\(\)\[\]\"\']+(?:\/[^\s\#\|\(\)\[\]\"\']+)*"#)
        .arg("--no-filename")
        .arg(directory)
        .stdout(Stdio::piped())
        .spawn()
        .context("rgコマンドの実行に失敗")?;

    let stdout = command.stdout.context("cant read from rg process")?;
    let reader = BufReader::new(stdout);
    Ok(reader
        .lines()
        .map(|line| line.map(|s| s.trim().to_string())))
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
#[command(about = "Get Tags from vault")]
struct Args {
    /// Path to the Obsidian vault
    #[arg(short, long)]
    path: Option<String>,

    #[arg(short, long, value_name = "in_content")]
    rg: bool,
}

use rayon::prelude::*;

fn collect_tags(paths: &Vec<PathBuf>) -> Result<HashSet<String>> {
    let result = paths
        .into_par_iter()
        .filter_map(|path| load_tags(path).ok())
        .flatten()
        .collect();
    Ok(result)
}

fn collect_paths(root: &Path) -> Vec<PathBuf> {
    let paths: Vec<_> = WalkDir::new(root)
        .into_iter()
        .filter_map(|entry| entry.ok().map(|e| e.path().to_path_buf()))
        .filter(|path| path.is_file() && path.extension().map_or(false, |ext| ext == "md"))
        .collect();
    paths
}

fn main() -> anyhow::Result<()> {
    env_logger::init();

    dotenv().ok();

    let args = Args::parse();

    let vault_path = if let Some(path) = args.path {
        path
    } else {
        env::var_os("OBSIDIAN_VAULT_PATH")
            .map(|x| x.into_string().expect("failed to convert path"))
            .ok_or(anyhow!("OBSIDIAN_VAULT_PATH not set"))?
    };

    let vault_path: PathBuf = expanduser(vault_path)?;
    let files = collect_paths(&vault_path);

    let mut collected_tags = collect_tags(&files)?;

    if args.rg {
        let tags = collect_obsidian_tags(vault_path.to_str().expect("utf8 error"))?;
        tags.into_iter().for_each(|tag| match tag {
            Ok(tag) => {
                collected_tags.insert(tag);
            }
            Err(e) => error!("error occured: {:?}", e),
        });
    }

    for tag in collected_tags {
        println!("{}", remove_hash(&tag));
    }

    Ok(())
}

fn remove_hash(s: &str) -> &str {
    s.trim_start_matches('#')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_cli() {
        use clap::CommandFactory;
        Args::command().debug_assert();
    }
}
