//! How far each part of a round's output can be trusted.

use serde::Serialize;

/// Trust level of one output field, best first.
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    /// Read directly from the replay.
    #[default]
    Decoded,
    /// Worked out from other data rather than read (for example the round
    /// winner before Y9S4).
    Inferred,
    /// Read, but some of it failed or looks incomplete.
    Partial,
    /// Expected, but nothing was found.
    Missing,
    /// This replay's version does not carry the field, or this parser cannot
    /// decode it for that version.
    NotInVersion,
    /// Not read because of the read mode (`--partial`).
    Skipped,
}

impl Status {
    /// Whether a consumer can use the value as is.
    pub fn trusted(self) -> bool {
        matches!(self, Status::Decoded | Status::Inferred)
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FieldReport {
    pub field: &'static str,
    pub status: Status,
    /// Entries decoded for this field.
    pub count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DecodeReport {
    /// True when every field is decoded, inferred, not in this version, or
    /// skipped: false marks a fault, not something left unread.
    pub trusted: bool,
    pub fields: Vec<FieldReport>,
    /// Problems not tied to one field.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl DecodeReport {
    pub fn field(&mut self, field: &'static str, status: Status, count: usize) -> &mut FieldReport {
        self.fields.push(FieldReport {
            field,
            status,
            count,
            warnings: Vec::new(),
        });
        self.fields.last_mut().expect("just pushed")
    }

    pub fn get(&self, field: &str) -> Option<&FieldReport> {
        self.fields.iter().find(|f| f.field == field)
    }

    pub fn get_mut(&mut self, field: &str) -> Option<&mut FieldReport> {
        self.fields.iter_mut().find(|f| f.field == field)
    }

    pub fn finish(&mut self) {
        self.trusted = self.fields.iter().all(|f| {
            f.status.trusted() || matches!(f.status, Status::NotInVersion | Status::Skipped)
        });
    }
}

impl FieldReport {
    pub fn warn(&mut self, message: impl Into<String>) -> &mut Self {
        self.warnings.push(message.into());
        self
    }

    /// Lowers the status to `status` if that is worse.
    pub fn at_most(&mut self, status: Status) -> &mut Self {
        self.status = self.status.max(status);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `trusted` marks faults: a field left unread (a partial read, a custom
    /// game's party) is not one.
    #[test]
    fn a_skipped_field_does_not_lower_trust() {
        let mut r = DecodeReport::default();
        r.field("players", Status::Decoded, 10);
        r.field("party", Status::Skipped, 0);
        r.field("feedbackMessages", Status::NotInVersion, 0);
        r.finish();
        assert!(r.trusted);
        r.field("kills", Status::Partial, 3);
        r.finish();
        assert!(!r.trusted);
    }
}
