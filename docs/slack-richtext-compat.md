# Slack rich-text compatibility matrix

Status values:
- `full`: round-trips from fixture JSON to deterministic IRC output in golden tests.
- `partial`: supported with known behavior gaps; requires written justification.
- `unsupported`: not rendered yet; requires written justification.

| Fixture | Blocks rich-text feature | Status | Justification |
| --- | --- | --- | --- |
| `section_text` | `rich_text_section` + plain `text` | full | |
| `emoji_link_date` | `emoji`, `link`, `date` inline elements | partial | Date rendering intentionally uses Slack `fallback` text only; it does not evaluate Slack date formatting templates. |
| `mentions` | `user`, `channel`, `broadcast`, `team`, `usergroup` mentions | partial | Team and usergroup mentions currently render deterministic unknown-ID placeholders because name resolution is not implemented in the resolver API. |
| `quote` | `rich_text_quote` | full | |
| `preformatted` | `rich_text_preformatted` | full | |
| `list_bullet` | `rich_text_list` (`style=bullet`) | full | |
| `list_ordered_indented` | `rich_text_list` (`style=ordered`, `indent`) | full | |
| `list_in_quote` | list nested inside quote | full | |
| `quote_in_list` | quote nested inside list item | full | |
| `code_within_list` | preformatted block nested inside list item | full | |
| `(no fixture)` | `color` style on rich-text `text` | unsupported | Slack color spans are parsed but not represented in IRC output; IRC has no color mapping implementation in renderer today. |
