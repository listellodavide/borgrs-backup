//! Exclusion list management for backup operations
//!
//! Supports glob patterns, regex patterns, and various path-based exclusions
//! similar to .gitignore files.

use crate::error::{BorgError, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, instrument};

/// Pattern type for exclusions
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PatternType {
    /// Shell-style glob pattern (e.g., "*.log", "**/cache/**")
    Glob,
    /// Literal path prefix
    PathPrefix,
    /// Regular expression pattern
    Regex,
    /// Fnmatch-style pattern (Borg compatibility)
    FnMatch,
}

/// A single exclusion pattern
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExclusionPattern {
    /// The pattern string
    pub pattern: String,
    /// Type of pattern
    pub pattern_type: PatternType,
    /// Whether this is a directory-only pattern
    pub directory_only: bool,
    /// Comment/description for this pattern
    pub comment: Option<String>,
}

impl ExclusionPattern {
    /// Create a new glob pattern
    pub fn glob(pattern: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            pattern_type: PatternType::Glob,
            directory_only: false,
            comment: None,
        }
    }

    /// Create a new path prefix pattern
    pub fn path_prefix(path: impl Into<String>) -> Self {
        Self {
            pattern: path.into(),
            pattern_type: PatternType::PathPrefix,
            directory_only: false,
            comment: None,
        }
    }

    /// Mark this pattern as directory-only
    pub fn directory_only(mut self) -> Self {
        self.directory_only = true;
        self
    }

    /// Add a comment to this pattern
    pub fn with_comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }
}

/// Compiled exclusion list for efficient matching
pub struct ExclusionList {
    /// Original patterns for serialization
    patterns: Vec<ExclusionPattern>,
    /// Compiled glob matcher
    glob_set: GlobSet,
    /// Path prefixes for quick matching
    path_prefixes: Vec<PathBuf>,
    /// Regex patterns (compiled on-demand)
    #[allow(dead_code)]
    regex_patterns: Vec<regex::Regex>,
    /// Statistics
    stats: ExclusionStats,
}

/// Statistics for exclusion operations
#[derive(Debug, Default, Clone)]
pub struct ExclusionStats {
    /// Total paths checked
    pub paths_checked: u64,
    /// Paths excluded
    pub paths_excluded: u64,
    /// Paths included
    pub paths_included: u64,
}

impl ExclusionStats {
    /// Get exclusion percentage
    pub fn exclusion_rate(&self) -> f64 {
        if self.paths_checked == 0 {
            0.0
        } else {
            (self.paths_excluded as f64 / self.paths_checked as f64) * 100.0
        }
    }
}

impl ExclusionList {
    /// Create a new empty exclusion list
    pub fn new() -> Self {
        Self {
            patterns: Vec::new(),
            glob_set: GlobSet::empty(),
            path_prefixes: Vec::new(),
            regex_patterns: Vec::new(),
            stats: ExclusionStats::default(),
        }
    }

    /// Create an exclusion list from patterns
    pub fn from_patterns(patterns: Vec<ExclusionPattern>) -> Result<Self> {
        let mut builder = GlobSetBuilder::new();
        let mut path_prefixes = Vec::new();
        let mut regex_patterns = Vec::new();

        for pattern in &patterns {
            match pattern.pattern_type {
                PatternType::Glob | PatternType::FnMatch => {
                    let glob = Glob::new(&pattern.pattern)?;
                    builder.add(glob);
                }
                PatternType::PathPrefix => {
                    path_prefixes.push(PathBuf::from(&pattern.pattern));
                }
                PatternType::Regex => {
                    let regex = regex::Regex::new(&pattern.pattern)
                        .map_err(|e| BorgError::ExclusionPattern(e.to_string()))?;
                    regex_patterns.push(regex);
                }
            }
        }

        let glob_set = builder.build()?;

        Ok(Self {
            patterns,
            glob_set,
            path_prefixes,
            regex_patterns,
            stats: ExclusionStats::default(),
        })
    }

    /// Load exclusion patterns from a file (one pattern per line, # for comments)
    #[instrument]
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path).map_err(|e| {
            BorgError::ExclusionPattern(format!("Failed to read exclusion file: {}", e))
        })?;

        Self::from_string(&content)
    }

    /// Parse exclusion patterns from a string
    pub fn from_string(content: &str) -> Result<Self> {
        let patterns: Vec<ExclusionPattern> = content
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                
                // Skip empty lines and comments
                if line.is_empty() || line.starts_with('#') {
                    return None;
                }

                // Parse pattern type prefix
                let (pattern_type, pattern) = if let Some(p) = line.strip_prefix("pp:") {
                    (PatternType::PathPrefix, p.to_string())
                } else if let Some(p) = line.strip_prefix("re:") {
                    (PatternType::Regex, p.to_string())
                } else if let Some(p) = line.strip_prefix("fm:") {
                    (PatternType::FnMatch, p.to_string())
                } else if let Some(p) = line.strip_prefix("gl:") {
                    (PatternType::Glob, p.to_string())
                } else {
                    // Default to glob
                    (PatternType::Glob, line.to_string())
                };

                Some(ExclusionPattern {
                    pattern,
                    pattern_type,
                    directory_only: false,
                    comment: None,
                })
            })
            .collect();

        debug!("Parsed {} exclusion patterns", patterns.len());
        Self::from_patterns(patterns)
    }

    /// Add a single pattern
    pub fn add_pattern(&mut self, pattern: ExclusionPattern) -> Result<()> {
        self.patterns.push(pattern);
        // Rebuild the compiled matchers
        *self = Self::from_patterns(self.patterns.clone())?;
        Ok(())
    }

    /// Check if a path should be excluded
    #[instrument(skip(self), fields(excluded))]
    pub fn is_excluded(&mut self, path: &Path, is_directory: bool) -> bool {
        self.stats.paths_checked += 1;

        let path_str = path.to_string_lossy();

        // Check path prefixes
        for prefix in &self.path_prefixes {
            if path.starts_with(prefix) {
                debug!(path = %path_str, reason = "path_prefix", "Excluded");
                self.stats.paths_excluded += 1;
                return true;
            }
        }

        // Check glob patterns
        if self.glob_set.is_match(path) {
            // Verify directory-only constraint if applicable
            let pattern_idx = self.glob_set.matches(path);
            for idx in pattern_idx {
                if idx < self.patterns.len() {
                    let pattern = &self.patterns[idx];
                    if pattern.directory_only && !is_directory {
                        continue;
                    }
                    debug!(
                        path = %path_str,
                        pattern = %pattern.pattern,
                        reason = "glob",
                        "Excluded"
                    );
                    self.stats.paths_excluded += 1;
                    return true;
                }
            }
        }

        // Check regex patterns
        for regex in &self.regex_patterns {
            if regex.is_match(&path_str) {
                debug!(path = %path_str, reason = "regex", "Excluded");
                self.stats.paths_excluded += 1;
                return true;
            }
        }

        self.stats.paths_included += 1;
        false
    }

    /// Get the current patterns
    pub fn patterns(&self) -> &[ExclusionPattern] {
        &self.patterns
    }

    /// Get statistics
    pub fn stats(&self) -> &ExclusionStats {
        &self.stats
    }

    /// Reset statistics
    pub fn reset_stats(&mut self) {
        self.stats = ExclusionStats::default();
    }

    /// Serialize patterns to a string (for saving to file)
    pub fn to_string(&self) -> String {
        let mut output = String::new();
        output.push_str("# Borg-Rust Exclusion Patterns\n");
        output.push_str("# Prefix patterns with: gl: (glob), pp: (path prefix), re: (regex), fm: (fnmatch)\n\n");

        for pattern in &self.patterns {
            if let Some(comment) = &pattern.comment {
                output.push_str(&format!("# {}\n", comment));
            }
            let prefix = match pattern.pattern_type {
                PatternType::Glob => "gl:",
                PatternType::PathPrefix => "pp:",
                PatternType::Regex => "re:",
                PatternType::FnMatch => "fm:",
            };
            output.push_str(&format!("{}{}\n", prefix, pattern.pattern));
        }

        output
    }
}

impl Default for ExclusionList {
    fn default() -> Self {
        Self::new()
    }
}

/// Common exclusion patterns for different use cases
pub struct CommonExclusions;

impl CommonExclusions {
    /// Get patterns for excluding common cache directories
    pub fn caches() -> Vec<ExclusionPattern> {
        vec![
            ExclusionPattern::glob("**/.cache/**").with_comment("XDG cache directory"),
            ExclusionPattern::glob("**/cache/**").with_comment("Generic cache directories"),
            ExclusionPattern::glob("**/__pycache__/**").with_comment("Python bytecode cache"),
            ExclusionPattern::glob("**/node_modules/**").with_comment("Node.js dependencies"),
            ExclusionPattern::glob("**/.npm/**").with_comment("NPM cache"),
            ExclusionPattern::glob("**/.cargo/registry/**").with_comment("Cargo registry cache"),
            ExclusionPattern::glob("**/target/debug/**").with_comment("Rust debug builds"),
            ExclusionPattern::glob("**/target/release/**").with_comment("Rust release builds"),
        ]
    }

    /// Get patterns for excluding temporary files
    pub fn temp_files() -> Vec<ExclusionPattern> {
        vec![
            ExclusionPattern::glob("**/*.tmp").with_comment("Temporary files"),
            ExclusionPattern::glob("**/*.temp").with_comment("Temporary files"),
            ExclusionPattern::glob("**/*.swp").with_comment("Vim swap files"),
            ExclusionPattern::glob("**/*.swo").with_comment("Vim swap files"),
            ExclusionPattern::glob("**/*~").with_comment("Backup files"),
            ExclusionPattern::glob("**/.*.swp").with_comment("Hidden vim swap files"),
            ExclusionPattern::glob("/tmp/**").with_comment("System temp directory"),
            ExclusionPattern::glob("/var/tmp/**").with_comment("Variable temp directory"),
        ]
    }

    /// Get patterns for excluding system directories
    pub fn system() -> Vec<ExclusionPattern> {
        vec![
            ExclusionPattern::glob("/proc/**").with_comment("Process filesystem"),
            ExclusionPattern::glob("/sys/**").with_comment("Sysfs"),
            ExclusionPattern::glob("/dev/**").with_comment("Device files"),
            ExclusionPattern::glob("/run/**").with_comment("Runtime data"),
            ExclusionPattern::glob("/var/run/**").with_comment("Runtime data (legacy)"),
            ExclusionPattern::glob("/lost+found/**").with_comment("Filesystem recovery"),
        ]
    }

    /// Get patterns for excluding version control metadata
    pub fn vcs() -> Vec<ExclusionPattern> {
        vec![
            ExclusionPattern::glob("**/.git/**").with_comment("Git repository data"),
            ExclusionPattern::glob("**/.svn/**").with_comment("Subversion data"),
            ExclusionPattern::glob("**/.hg/**").with_comment("Mercurial data"),
            ExclusionPattern::glob("**/.bzr/**").with_comment("Bazaar data"),
        ]
    }

    /// Get patterns for excluding log files
    pub fn logs() -> Vec<ExclusionPattern> {
        vec![
            ExclusionPattern::glob("**/*.log").with_comment("Log files"),
            ExclusionPattern::glob("**/logs/**").with_comment("Log directories"),
            ExclusionPattern::glob("/var/log/**").with_comment("System logs"),
        ]
    }

    /// Get a comprehensive default exclusion list
    pub fn defaults() -> Vec<ExclusionPattern> {
        let mut patterns = Vec::new();
        patterns.extend(Self::caches());
        patterns.extend(Self::temp_files());
        patterns.extend(Self::system());
        patterns
    }
}

// Add regex crate
mod regex {
    pub struct Regex(regex_lite::Regex);

    impl Regex {
        pub fn new(pattern: &str) -> std::result::Result<Self, String> {
            regex_lite::Regex::new(pattern)
                .map(Regex)
                .map_err(|e| e.to_string())
        }

        pub fn is_match(&self, text: &str) -> bool {
            self.0.is_match(text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_glob_exclusion() {
        let patterns = vec![ExclusionPattern::glob("*.log")];
        let mut list = ExclusionList::from_patterns(patterns).unwrap();

        assert!(list.is_excluded(Path::new("app.log"), false));
        assert!(list.is_excluded(Path::new("/var/log/system.log"), false));
        assert!(!list.is_excluded(Path::new("app.txt"), false));
    }

    #[test]
    fn test_path_prefix_exclusion() {
        let patterns = vec![ExclusionPattern::path_prefix("/home/user/.cache")];
        let mut list = ExclusionList::from_patterns(patterns).unwrap();

        assert!(list.is_excluded(Path::new("/home/user/.cache/app"), true));
        assert!(list.is_excluded(Path::new("/home/user/.cache/file.txt"), false));
        assert!(!list.is_excluded(Path::new("/home/user/documents"), true));
    }

    #[test]
    fn test_recursive_glob() {
        let patterns = vec![ExclusionPattern::glob("**/node_modules/**")];
        let mut list = ExclusionList::from_patterns(patterns).unwrap();

        assert!(list.is_excluded(Path::new("/project/node_modules/package"), true));
        assert!(list.is_excluded(Path::new("/deep/path/node_modules/file.js"), false));
        assert!(!list.is_excluded(Path::new("/project/src/app.js"), false));
    }

    #[test]
    fn test_parse_from_string() {
        let content = r#"
# Comment line
*.log
pp:/tmp
# Another comment
re:.*\.bak$
"#;
        let mut list = ExclusionList::from_string(content).unwrap();
        
        assert_eq!(list.patterns().len(), 3);
        assert!(list.is_excluded(Path::new("file.log"), false));
        assert!(list.is_excluded(Path::new("/tmp/file"), false));
    }

    #[test]
    fn test_directory_only() {
        let patterns = vec![ExclusionPattern::glob("**/build").directory_only()];
        let mut list = ExclusionList::from_patterns(patterns).unwrap();

        // Directory should be excluded
        assert!(list.is_excluded(Path::new("/project/build"), true));
        // File named "build" should not be excluded
        assert!(!list.is_excluded(Path::new("/project/build"), false));
    }

    #[test]
    fn test_statistics() {
        let patterns = vec![ExclusionPattern::glob("*.log")];
        let mut list = ExclusionList::from_patterns(patterns).unwrap();

        list.is_excluded(Path::new("app.log"), false);
        list.is_excluded(Path::new("app.txt"), false);
        list.is_excluded(Path::new("system.log"), false);

        let stats = list.stats();
        assert_eq!(stats.paths_checked, 3);
        assert_eq!(stats.paths_excluded, 2);
        assert_eq!(stats.paths_included, 1);
    }

    #[test]
    fn test_common_exclusions() {
        let patterns = CommonExclusions::defaults();
        let mut list = ExclusionList::from_patterns(patterns).unwrap();

        assert!(list.is_excluded(Path::new("/proc/1/status"), false));
        assert!(list.is_excluded(Path::new("/home/user/.cache/thumbnails"), true));
        assert!(list.is_excluded(Path::new("/project/__pycache__/module.pyc"), false));
    }
}
