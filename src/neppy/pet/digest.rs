//! Deterministic digest assembly. Titles come from untrusted content, so every
//! interpolated string is markdown-escaped (links and images render inert).

use std::collections::HashSet;

use chrono::{DateTime, Duration, TimeZone, Utc};

use super::types::{PetNote, PetNoteKind, DIGEST_MAX_ITEMS};

/// What [`build_digest`] produced. Every id in `shown`, `overflow` and
/// `withheld` is marked digested so the next digest starts fresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DigestBuild {
    pub body_md: String,
    /// Listed in the body (at most [`DIGEST_MAX_ITEMS`]).
    pub shown: Vec<String>,
    /// Over the cap — summarised as "N more in Neppy".
    pub overflow: Vec<String>,
    /// Flagged for possible prompt injection — counted, never listed.
    pub withheld: Vec<String>,
}

impl DigestBuild {
    pub(crate) fn all_ids(&self) -> Vec<String> {
        self.shown
            .iter()
            .chain(&self.overflow)
            .chain(&self.withheld)
            .cloned()
            .collect()
    }
}

/// Escape markdown control characters so untrusted text cannot form links,
/// images, emphasis, headings, HTML or tables. Bare URLs are neutralised too
/// (GFM-style renderers autolink them): the scheme separator, `www.` hosts and
/// `@` of an address are backslash-escaped so no autolink can form.
pub(crate) fn escape_md(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for (i, &c) in chars.iter().enumerate() {
        let www_dot = c == '.'
            && i >= 3
            && chars[i - 3..i].iter().all(|w| w.eq_ignore_ascii_case(&'w'))
            && (i == 3 || !chars[i - 4].is_alphanumeric());
        let scheme_sep =
            c == ':' && chars.get(i + 1) == Some(&'/') && chars.get(i + 2) == Some(&'/');
        if www_dot
            || scheme_sep
            || c == '@'
            || matches!(
                c,
                '\\' | '*' | '_' | '`' | '[' | ']' | '(' | ')' | '#' | '<' | '>' | '!' | '|'
            )
        {
            out.push('\\');
        }
        if c.is_control() {
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    NeedsYou,
    ComingUp,
    Fyi,
}

fn section_for(note: &PetNote, proposal_notes: &HashSet<String>, now: DateTime<Utc>) -> Section {
    if matches!(note.kind, PetNoteKind::Request | PetNoteKind::Deadline)
        || proposal_notes.contains(&note.id)
    {
        Section::NeedsYou
    } else if note.kind == PetNoteKind::Meeting
        || note.due_at.is_some_and(|d| d <= now + Duration::hours(72))
    {
        Section::ComingUp
    } else {
        Section::Fyi
    }
}

/// Assemble the digest body from digest candidates. Returns `None` when there
/// is nothing at all (no candidates, withheld included).
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_digest<Tz: TimeZone>(
    pet_name: &str,
    notes: &[PetNote],
    proposal_notes: &HashSet<String>,
    pending_proposals: usize,
    pending_approvals: usize,
    now: DateTime<Utc>,
    tz: &Tz,
) -> Option<DigestBuild>
where
    Tz::Offset: std::fmt::Display,
{
    if notes.is_empty() {
        return None;
    }
    let (withheld, mut listable): (Vec<&PetNote>, Vec<&PetNote>) =
        notes.iter().partition(|n| n.injection_flagged);
    listable.sort_by(|a, b| {
        b.score
            .unwrap_or(0)
            .cmp(&a.score.unwrap_or(0))
            .then_with(|| match (a.due_at, b.due_at) {
                (Some(x), Some(y)) => x.cmp(&y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then(a.created_at.cmp(&b.created_at))
            .then(a.id.cmp(&b.id))
    });
    let shown: Vec<&PetNote> = listable.iter().take(DIGEST_MAX_ITEMS).copied().collect();
    let overflow: Vec<&PetNote> = listable.iter().skip(DIGEST_MAX_ITEMS).copied().collect();

    let local_now = now.with_timezone(tz);
    let mut body = format!(
        "**{}: your digest for {}**",
        escape_md(pet_name),
        local_now.format("%A %-d %b")
    );
    for (section, heading) in [
        (Section::NeedsYou, "Needs you"),
        (Section::ComingUp, "Coming up"),
        (Section::Fyi, "FYI"),
    ] {
        let items: Vec<&&PetNote> = shown
            .iter()
            .filter(|n| section_for(n, proposal_notes, now) == section)
            .collect();
        if items.is_empty() {
            continue;
        }
        body.push_str(&format!("\n\n**{heading}**"));
        for note in items {
            let due = note
                .due_at
                .map(|d| format!(", due {}", d.with_timezone(tz).format("%a %H:%M")))
                .unwrap_or_default();
            body.push_str(&format!(
                "\n- {} ({}{})",
                escape_md(&note.title),
                note.source.as_str(),
                due
            ));
        }
    }

    let mut footer = Vec::new();
    if !overflow.is_empty() {
        footer.push(format!("{} more in Neppy", overflow.len()));
    }
    if pending_proposals > 0 {
        footer.push(format!(
            "{pending_proposals} suggestion(s) waiting in your Pet inbox"
        ));
    }
    if pending_approvals > 0 {
        footer.push(format!("{pending_approvals} approval(s) waiting"));
    }
    if !withheld.is_empty() {
        footer.push(format!(
            "{} note(s) withheld for review (possible prompt injection)",
            withheld.len()
        ));
    }
    if !footer.is_empty() {
        body.push_str("\n\n");
        body.push_str(&footer.join("\n"));
    }
    log::debug!(
        "[pet::digest] built shown={} overflow={} withheld={}",
        shown.len(),
        overflow.len(),
        withheld.len()
    );
    Some(DigestBuild {
        body_md: body,
        shown: shown.iter().map(|n| n.id.clone()).collect(),
        overflow: overflow.iter().map(|n| n.id.clone()).collect(),
        withheld: withheld.iter().map(|n| n.id.clone()).collect(),
    })
}
