//! Parsing a docstring section that documents parameters one bullet each, such as `# Type
//! Parameters` and `# Parameters`, and comparing its bullets with an item's parameters.

use crate::diagnostic::Position;
use crate::doc::doc_lines::DocLine;
use crate::doc::links;
use crate::doc::links::LinkDefinitions;
use crate::doc::links::ReferenceKind;
use crate::doc::section;

/// The section that documents an item's generic parameters.
pub const TYPE_PARAMETERS: ParamSectionKind = ParamSectionKind {
    title: "Type Parameters",
    parameter_noun: "generic parameter",
};

/// The section that documents a trait method's parameters.
pub const PARAMETERS: ParamSectionKind = ParamSectionKind {
    title: "Parameters",
    parameter_noun: "parameter",
};

/// A kind of section that documents parameters one bullet each.
#[derive(Debug, Clone, Copy)]
pub struct ParamSectionKind {
    /// The section's header title, such as `Type Parameters`.
    title: &'static str,

    /// What the section's bullets document, as diagnostics name it, such as `generic parameter`.
    parameter_noun: &'static str,
}

impl ParamSectionKind {
    /// Finds and parses the section with this kind's title, as [`section::find`] selects it.
    ///
    /// The section's top-level bullets form lists, each starting at the first bullet or at a bullet
    /// that follows prose. A list that follows prose and has no bullet naming a parameter belongs
    /// to that prose, so its bullets are dropped.
    ///
    /// # Returns
    ///
    /// The parsed section, or [`None`] if the docstring has no such section.
    pub fn parse(self, doc_lines: &[DocLine]) -> Option<ParamSection> {
        let doc_section = section::find(doc_lines, self.title)?;
        let texts: Vec<&str> = doc_lines.iter().map(|line| line.text.as_str()).collect();
        let definitions = LinkDefinitions::collect(&texts);
        let bullets = doc_section
            .content(doc_lines, &definitions)
            .bullets
            .chunk_by(|_, next| !next.follows_prose)
            .flat_map(|list| {
                let param_bullets: Vec<ParamBullet> = list
                    .iter()
                    .map(|bullet| parse_bullet(&bullet.text, bullet.position, &definitions))
                    .collect();
                let is_prose_list = list[0].follows_prose
                    && param_bullets
                        .iter()
                        .all(|bullet| bullet.form == BulletForm::Nameless);
                if is_prose_list {
                    Vec::new()
                } else {
                    param_bullets
                }
            })
            .collect();
        Some(ParamSection {
            header: doc_section.header,
            bullets,
            duplicate_headers: doc_section.duplicate_headers,
        })
    }

    /// # Returns
    ///
    /// The message for a documented `item`, such as ``fn `load` ``, that has the parameters `names`
    /// but no section of this kind.
    pub fn missing_section_message(self, item: &str, names: &[String]) -> String {
        format!(
            "{item} has {noun}s but no `# {title}` section; add one with a bullet for each of {}",
            code_list(names),
            noun = self.parameter_noun,
            title = self.title,
        )
    }

    /// # Returns
    ///
    /// The message of a section of this kind in the docstring of `item`, which has no parameters
    /// for it to document.
    pub fn no_parameters_message(self, item: &str) -> String {
        format!(
            "{item} has no {}s; remove the `# {}` section",
            self.parameter_noun, self.title
        )
    }

    /// # Returns
    ///
    /// The message of a [`ParamFinding`] of `kind` in the section of this kind of `item`.
    pub fn finding_message(self, kind: &ParamFindingKind, item: &str) -> String {
        let Self {
            title,
            parameter_noun,
        } = self;
        match kind {
            ParamFindingKind::DuplicateHeader => {
                format!(
                    "the docstring has a second `# {title}` section at the same level; merge them"
                )
            }
            ParamFindingKind::NamelessBullet => format!(
                "this `# {title}` bullet doesn't start with the {parameter_noun} it documents; \
                 write \"* `name` - description\""
            ),
            ParamFindingKind::NonCanonicalBullet { name } => {
                format!("write this `# {title}` bullet as \"* `{name}` - description\"")
            }
            ParamFindingKind::UnknownName { name } => format!(
                "`# {title}` documents `{name}`, which isn't a {parameter_noun} of {item}; remove \
                 the bullet"
            ),
            ParamFindingKind::DuplicateBullet { name, first_line } => {
                format!("`{name}` is already documented on line {first_line}; remove this bullet")
            }
            ParamFindingKind::Missing { name } => {
                format!("`# {title}` doesn't document `{name}`; add \"* `{name}` - ...\"")
            }
        }
    }
}

/// A section of a docstring whose top-level bullets each document one parameter.
#[derive(Debug, PartialEq, Eq)]
pub struct ParamSection {
    /// The position of the section's header.
    pub header: Position,

    /// The section's top-level bullets, in source order. Sub-bullets belong to their bullet.
    bullets: Vec<ParamBullet>,

    /// The positions of the later headers with the same title and level.
    duplicate_headers: Vec<Position>,
}

impl ParamSection {
    /// Compares the section with the parameters of its item, which must have at least one:
    ///
    /// * Each duplicate header is a [`ParamFindingKind::DuplicateHeader`].
    /// * A bullet without a name is a [`ParamFindingKind::NamelessBullet`].
    /// * A bullet whose name is neither in `required` nor in `accepted` is a
    ///   [`ParamFindingKind::UnknownName`].
    /// * A bullet whose name an earlier bullet already has is a
    ///   [`ParamFindingKind::DuplicateBullet`].
    /// * Any other bullet that isn't canonical is a [`ParamFindingKind::NonCanonicalBullet`]. It
    ///   still documents its parameter.
    /// * Each name of `required` that no bullet has is a [`ParamFindingKind::Missing`], at the
    ///   header.
    ///
    /// # Returns
    ///
    /// The findings, in that order, with the bullet findings in source order.
    pub fn compare(&self, required: &[String], accepted: &[String]) -> Vec<ParamFinding> {
        let mut findings: Vec<ParamFinding> = self
            .duplicate_headers
            .iter()
            .map(|position| ParamFinding {
                kind: ParamFindingKind::DuplicateHeader,
                position: *position,
            })
            .collect();
        let mut documented: Vec<(&str, usize)> = Vec::new();
        for bullet in &self.bullets {
            let name = match &bullet.form {
                BulletForm::Nameless => {
                    findings.push(ParamFinding {
                        kind: ParamFindingKind::NamelessBullet,
                        position: bullet.position,
                    });
                    continue;
                }
                BulletForm::NonCanonical { name } | BulletForm::Canonical { name } => name.as_str(),
            };
            let kind = if !required.iter().chain(accepted).any(|known| known == name) {
                Some(ParamFindingKind::UnknownName {
                    name: name.to_owned(),
                })
            } else if let Some((_, first_line)) = documented
                .iter()
                .find(|(documented_name, _)| *documented_name == name)
            {
                Some(ParamFindingKind::DuplicateBullet {
                    name: name.to_owned(),
                    first_line: *first_line,
                })
            } else {
                documented.push((name, bullet.position.line));
                matches!(bullet.form, BulletForm::NonCanonical { .. }).then(|| {
                    ParamFindingKind::NonCanonicalBullet {
                        name: name.to_owned(),
                    }
                })
            };
            findings.extend(kind.map(|kind| ParamFinding {
                kind,
                position: bullet.position,
            }));
        }
        findings.extend(
            required
                .iter()
                .filter(|name| {
                    !documented
                        .iter()
                        .any(|(documented_name, _)| documented_name == name)
                })
                .map(|name| ParamFinding {
                    kind: ParamFindingKind::Missing { name: name.clone() },
                    position: self.header,
                }),
        );
        findings
    }
}

/// A mismatch between a [`ParamSection`] and the parameters of its item.
#[derive(Debug)]
pub struct ParamFinding {
    pub kind: ParamFindingKind,
    pub position: Position,
}

/// The kinds of [`ParamFinding`].
#[derive(Debug)]
pub enum ParamFindingKind {
    /// A second header with the section's title at the section's level.
    DuplicateHeader,

    /// A bullet that doesn't start with a code span or a code link.
    NamelessBullet,

    /// A bullet that names a parameter, but not in the canonical form.
    NonCanonicalBullet { name: String },

    /// A bullet that names something that isn't a parameter of the item.
    UnknownName { name: String },

    /// A bullet that names a parameter an earlier bullet on `first_line` already names.
    DuplicateBullet { name: String, first_line: usize },

    /// A parameter that no bullet names. It is reported at the section's header.
    Missing { name: String },
}

/// The separator between a bullet's name and its description in the canonical bullet form.
const CANONICAL_SEPARATOR: &str = " - ";

/// A top-level bullet of a [`ParamSection`].
#[derive(Debug, PartialEq, Eq)]
struct ParamBullet {
    form: BulletForm,

    /// The position of the bullet's first line.
    position: Position,
}

/// The name a [`ParamBullet`] documents, and whether the bullet is in the canonical form: a bare
/// code span with the name, then [`CANONICAL_SEPARATOR`], then a non-blank description.
#[derive(Debug, PartialEq, Eq)]
enum BulletForm {
    /// The bullet starts with neither a code span nor a code link.
    Nameless,

    /// The bullet starts with a code span or a code link whose code is `name`, such as `T`, `'a` or
    /// `job_id`, but isn't in the canonical form.
    NonCanonical { name: String },

    /// The bullet is in the canonical form, and its code span is `name`.
    Canonical { name: String },
}

/// Parses the joined text of one top-level bullet at `position`.
///
/// # Returns
///
/// The bullet's name and form.
fn parse_bullet(text: &str, position: Position, definitions: &LinkDefinitions) -> ParamBullet {
    let Some(first_reference) = links::references(text, definitions)
        .into_iter()
        .next()
        .filter(|reference| reference.start == 0)
    else {
        return ParamBullet {
            form: BulletForm::Nameless,
            position,
        };
    };
    let name = first_reference.path_text;
    let is_canonical = first_reference.kind == ReferenceKind::BareCode
        && text[first_reference.end..]
            .strip_prefix(CANONICAL_SEPARATOR)
            .is_some_and(|description| !description.trim().is_empty());
    let form = if is_canonical {
        BulletForm::Canonical { name }
    } else {
        BulletForm::NonCanonical { name }
    };
    ParamBullet { form, position }
}

/// # Returns
///
/// `names` as code spans separated by `, `, such as `` `'a`, `T` ``.
fn code_list(names: &[String]) -> String {
    names
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc_lines(text: &str) -> Vec<DocLine> {
        text.lines()
            .enumerate()
            .map(|(index, line)| DocLine {
                text: line.to_owned(),
                position: Position {
                    line: index + 1,
                    column: 5,
                },
            })
            .collect()
    }

    fn bullets(text: &str, kind: ParamSectionKind) -> Vec<(BulletForm, usize)> {
        kind.parse(&doc_lines(text))
            .expect("a section")
            .bullets
            .into_iter()
            .map(|bullet| (bullet.form, bullet.position.line))
            .collect()
    }

    fn canonical(name: &str) -> BulletForm {
        BulletForm::Canonical {
            name: name.to_owned(),
        }
    }

    fn non_canonical(name: &str) -> BulletForm {
        BulletForm::NonCanonical {
            name: name.to_owned(),
        }
    }

    #[test]
    fn canonical_bullets_name_type_lifetime_and_value_parameters() {
        let text = "Summary.\n\n# Type Parameters\n\n* `StorageType` - The storage.\n* \
                    `'cache_lifetime` - The cache's\n  lifetime.\n* `LIMIT` - The limit.\n\n# \
                    Returns\n\n* `NotAParameter` - Outside the section.";
        assert_eq!(
            bullets(text, TYPE_PARAMETERS),
            [
                (canonical("StorageType"), 5),
                (canonical("'cache_lifetime"), 6),
                (canonical("LIMIT"), 8),
            ]
        );
        let section = TYPE_PARAMETERS.parse(&doc_lines(text)).expect("a section");
        assert_eq!(section.header, Position { line: 3, column: 5 });
        assert_eq!(section.duplicate_headers, []);
    }

    #[test]
    fn non_canonical_bullets_keep_their_name_when_they_start_with_code() {
        let text = "# Parameters\n\n* [`job_id`] - A link.\n* `em_id`: A colon.\n* `ip` -\n* \
                    `session_id` -- Two dashes.\n* The `task_id` - Prose first.\n* Plain text.\n  \
                    * `nested` - A sub-bullet.";
        assert_eq!(
            bullets(text, PARAMETERS),
            [
                (non_canonical("job_id"), 3),
                (non_canonical("em_id"), 4),
                (non_canonical("ip"), 5),
                (non_canonical("session_id"), 6),
                (BulletForm::Nameless, 7),
                (BulletForm::Nameless, 8),
            ]
        );
    }

    #[test]
    fn nameless_lists_after_prose_belong_to_the_prose() {
        let text = "# Type Parameters\n\nThe parameters are forwarded. Short names are used \
                    to:\n\n* Reduce verbosity.\n* Avoid wrapping.\n\nThey are:\n\n* `R` - The \
                    receiver.\n* The sender.\n\nNotes:\n\n* `Unknown` is a code \
                    span.\n\nRemarks:\n\n* A remark.";
        assert_eq!(
            bullets(text, TYPE_PARAMETERS),
            [
                (canonical("R"), 10),
                (BulletForm::Nameless, 11),
                (non_canonical("Unknown"), 15),
            ]
        );
    }

    #[test]
    fn description_may_continue_on_the_next_line() {
        let text = "# Parameters\n\n* `job_id` -\n  The owning job.";
        assert_eq!(bullets(text, PARAMETERS), [(canonical("job_id"), 3)]);
    }

    #[test]
    fn missing_sections_and_duplicate_headers_are_reported() {
        assert_eq!(PARAMETERS.parse(&doc_lines("# Returns\n\nA.")), None);
        assert_eq!(
            PARAMETERS.parse(&doc_lines("# Returns\n\n## Parameters\n\n* `a` - A.")),
            None
        );
        let section = PARAMETERS
            .parse(&doc_lines(
                "# Parameters\n\n* `a` - A.\n\n# Parameters\n\n* `b` - B.",
            ))
            .expect("a section");
        assert_eq!(section.bullets.len(), 1);
        assert_eq!(section.duplicate_headers, [Position { line: 5, column: 5 }]);
    }

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn compared(text: &str, required: &[&str], accepted: &[&str]) -> Vec<(usize, String)> {
        let section = PARAMETERS.parse(&doc_lines(text)).expect("a section");
        section
            .compare(&names(required), &names(accepted))
            .into_iter()
            .map(|finding| {
                (
                    finding.position.line,
                    PARAMETERS.finding_message(&finding.kind, "trait method `submit`"),
                )
            })
            .collect()
    }

    #[test]
    fn bullets_matching_the_parameters_have_no_findings() {
        let text = "# Parameters\n\n* `job_id` - The job.\n* `inputs` - The inputs.\n* `x` - A \
                    destructured binding.";
        assert_eq!(
            compared(text, &["inputs", "job_id"], &["x"]),
            Vec::<(usize, String)>::new()
        );
    }

    #[test]
    fn missing_unknown_and_duplicate_names_are_findings() {
        let text = "# Parameters\n\n* `job_id` - The job.\n* `stale` - Gone.\n* `job_id` - \
                    Again.\n* `stale` - Gone again.";
        assert_eq!(
            compared(text, &["job_id", "inputs"], &[]),
            [
                (
                    4,
                    "`# Parameters` documents `stale`, which isn't a parameter of trait method \
                     `submit`; remove the bullet"
                        .to_owned()
                ),
                (
                    5,
                    "`job_id` is already documented on line 3; remove this bullet".to_owned()
                ),
                (
                    6,
                    "`# Parameters` documents `stale`, which isn't a parameter of trait method \
                     `submit`; remove the bullet"
                        .to_owned()
                ),
                (
                    1,
                    "`# Parameters` doesn't document `inputs`; add \"* `inputs` - ...\"".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn malformed_bullets_and_duplicate_headers_are_findings() {
        let text = "# Parameters\n\n* [`job_id`]: The job.\n* The inputs.\n\n# Parameters\n\n* \
                    `inputs` - The inputs.";
        assert_eq!(
            compared(text, &["job_id", "inputs"], &[]),
            [
                (
                    6,
                    "the docstring has a second `# Parameters` section at the same level; merge \
                     them"
                        .to_owned()
                ),
                (
                    3,
                    "write this `# Parameters` bullet as \"* `job_id` - description\"".to_owned()
                ),
                (
                    4,
                    "this `# Parameters` bullet doesn't start with the parameter it documents; \
                     write \"* `name` - description\""
                        .to_owned()
                ),
                (
                    1,
                    "`# Parameters` doesn't document `inputs`; add \"* `inputs` - ...\"".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn section_level_messages_name_the_item_and_its_parameters() {
        assert_eq!(
            TYPE_PARAMETERS
                .missing_section_message("fn `load`", &names(&["'cache_lifetime", "StorageType"])),
            "fn `load` has generic parameters but no `# Type Parameters` section; add one with a \
             bullet for each of `'cache_lifetime`, `StorageType`"
        );
        assert_eq!(
            TYPE_PARAMETERS.no_parameters_message("struct `Job`"),
            "struct `Job` has no generic parameters; remove the `# Type Parameters` section"
        );
    }
}
