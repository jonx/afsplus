# Documentation Rules

How documentation is written in this repository, for the next agent or
contributor. [`tools/check-docs.py`](../tools/check-docs.py) enforces the mechanical part; the rest is
judgement, and this page is its reference. [AGENTS.md](../AGENTS.md) points
here; [CONTRIBUTING.md](../CONTRIBUTING.md) names the checker.

<!-- toc -->

- [1. Finished-state text](#1-finished-state-text)
- [2. Four kinds of document, one home per fact](#2-four-kinds-of-document-one-home-per-fact)
- [3. Per-document rules](#3-per-document-rules)
- [4. Navigation block](#4-navigation-block)
- [5. Tables of contents](#5-tables-of-contents)
- [6. Links](#6-links)
- [7. Checklist before committing](#7-checklist-before-committing)
- [8. The checker](#8-the-checker)

<!-- /toc -->

## 1. Finished-state text

Every document except the journal describes the finished state: what AFS+ is,
how it is built, what a procedure does, what a milestone requires. It never
narrates how the project got there. Words that narrate a journey belong in
[NOTES.md](../NOTES.md) or in an ADR's `Status:` line and nowhere else:

> now · currently · still · remains / remain · no longer · the latest ·
> was superseded by · so far · to date · at the moment · recently · previously

Plain English uses of these words ("the source is still present after a
failed rename") are fine; what is forbidden is progress narration ("the
allocator now loads pages on demand"). `tools/check-docs.py --journey` lists
the counts per file without failing, so the residue can be worked down.

When touching a document that still has journey text, convert the section
being touched to finished-state text and never append another paragraph of
narration after it.

## 2. Four kinds of document, one home per fact

| Kind | Answers | Home |
|---|---|---|
| State | where are we? | [implementation/milestones.md](../implementation/milestones.md) — the only status authority |
| Design | how is it built and why? | [docs/NN-*.md](README.md), [spec/](README.md#spec), [api/](README.md#api), [adr/](../adr/README.md) |
| Procedure | what do I run, what does it prove? | [testing/*.md](../testing/README.md), [tools/README.md](../tools/README.md), [tools/tools-spec.md](../tools/tools-spec.md), [CONTRIBUTING.md](../CONTRIBUTING.md) |
| History | what was decided, tried, delivered? | [adr/](../adr/README.md) (decisions), [NOTES.md](../NOTES.md) (journal) |
| Agent rules | how do agents work here? | [AGENTS.md](../AGENTS.md); [`CLAUDE.md`](../CLAUDE.md) only points to it |

A fact is written once, in its home. Every other mention is a link to that
home, never a paraphrase that can drift. Deliberately unresolved decisions
have their own home, [implementation/open-questions.md](../implementation/open-questions.md);
drafts awaiting team review have theirs, [proposals/](../proposals/README.md).

## 3. Per-document rules

- **README.md** is product-facing: what AFS+ is and is not, the developer
  contract, the architecture picture, how to build and test, the repository
  map, the documentation map, the license. Its `## Status` section is a table
  of **at most five rows**, each linking the milestone table.
- **implementation/milestones.md**: one row per milestone; the status cell is
  **one line** — `state — what passes; what is open` — with `Design` and
  `Test plan` link columns. Detail that does not fit the line goes to the
  test plan or design document the row links.
- **ROADMAP.md** carries the plan only: stages, order, exit criteria, and the
  milestones each stage feeds. It carries no `Status:` line.
- **ADR status vocabulary:** use `Accepted`, `Proposed`, `Partially accepted`,
  `Reopened` or `Superseded`. Omit approval attribution and implementation
  progress from status lines. Scope follows decision and relation text;
  qualification progress belongs in the milestone table.
- **ADRs** become immutable when they are accepted, not when they are written.
  A record whose `Status:` is `Proposed` is a draft under review and may be
  revised in place while its review thread is open; once its status records
  acceptance, only that `Status:` line and the relation lines `Supersedes:`,
  `Superseded by:`, `Amends:`, `Amended by:` may change, and a changed decision
  needs a new record. Numbers are never reused; [`adr/README.md`](../adr/README.md) is generated from these
  headers. Ideas no ADR supersedes formally are listed in
  [adr/README.md § Superseded directions](../adr/README.md#superseded-directions).
- **Design documents** (`docs/NN-*.md`, `spec/`, `api/`) carry no status, no
  dates and no "currently". Their numbers are permanent. Rewording a normative
  sentence follows the format- or API-change procedure in
  [CONTRIBUTING.md](../CONTRIBUTING.md); linking, adding a navigation block or
  a table of contents is not a change of meaning.
- **Test plans** (`testing/*.md`) state what a gate checks and which script or
  `cargo test` target runs it. They never record results.
- **tools/README.md** and **testing/README.md** have exactly one row per file
  in their directory; [`docs/README.md`](README.md) and [`implementation/README.md`](../implementation/README.md) list
  every document in theirs.
- **NOTES.md** is the only narrative document: newest entry first, headed
  `## YYYY-MM-DD — title`. Text removed from another document because it had
  no home goes here with its original location noted.
- **proposals/*.md** open with the navigation block and a "Target on
  acceptance" line naming where the document lands if accepted.
- Every new Markdown file follows the repository's header convention: shell
  and Python files carry an SPDX line; Markdown files start with a single
  `#` title.

## 4. Navigation block

Every `docs/*.md` (except this file and the index), every `testing/*.md` and
every `proposals/*.md` opens with this block right after the title:

```markdown
> **ADRs:** [ADR-009](../adr/ADR-009-journal.md), … · **Spec:** [invariants](../spec/invariants.md) ·
> **Tests:** [crash-testing](../testing/crash-testing.md) · **Milestones:** M04
```

`none` fills an empty field. The content is derived from what the document
cites and from the `Design`/`Test plan` columns of the milestone table; it
never invents a relation. Long ADR lists wrap onto further `> ` lines; the
block is one blockquote.

## 5. Tables of contents

Every Markdown file longer than 150 lines carries a generated table of
contents between `<!-- toc -->` and `<!-- /toc -->`, placed after the
introduction (before the first `##` heading). It is never hand-written:
`make toc` (or `tools/check-docs.py --write-toc`) inserts and regenerates it.
Short files get none.

## 6. Links

Repository paths are Markdown links, not backticked strings, whenever the
target exists. Links are relative to the citing file. Every `ADR-NNN` mention
must resolve to `adr/ADR-NNN-*.md`; every `MNN` milestone mention must be a
row of the milestone table.

## 7. Checklist before committing

- New ADR → `make adr-index` regenerates [`adr/README.md`](../adr/README.md); the design document
  it affects links it (navigation block).
- New tool or script → one row in [`tools/README.md`](../tools/README.md).
- New test plan → one row in [`testing/README.md`](../testing/README.md) with the milestone it feeds,
  and the milestone row links the plan.
- New design or implementation document → one row in [`docs/README.md`](README.md) or
  [`implementation/README.md`](../implementation/README.md), navigation block, index link.
- Status change → one cell of [`implementation/milestones.md`](../implementation/milestones.md).
- Something worth telling → an entry in [`NOTES.md`](../NOTES.md).
- Text removed from a document without a home → [`NOTES.md`](../NOTES.md), with the original
  location.
- `make check-docs` passes, and what it regenerated is staged: because
  files are staged explicitly rather than with `git add -A`, a generated
  index or table of contents is easily left dirty, so `git status` must be
  clean once the commit is made.

Progress labels in the start page, roadmap and implementation navigation are
generated views of milestone status. Use plain IDs for not started, visible
brackets for partial work and strikethrough for complete. This generated
navigation is an exception to the single-home status rule; edit the milestone
status cell and run `make toc`. Stage labels aggregate their finite contributing
milestones. Ongoing activities stay visible and are excluded from completion
calculations; an all-ongoing group receives no completion mark. Stage 0 represents
ongoing design review. M00 is the finite epoch-1 reader-format freeze in Stage F. Preserve stable
heading anchors and clickable links.

Completed individual entries in roadmap stage lists and implementation-plan
deliverable/acceptance lists must also be struck through. Record each marked
item's status and evidence in
[the item table](../implementation/milestones.md#individual-list-item-completion).
A stable hidden progress marker binds the list entry to that record; the
navigation generator adds or removes strikethrough and fills the item table's
Task and Stage / phase columns from the source list and heading. These readable
descriptions and navigation links must stay synchronized with the lists. Mark an item complete only
when its own stated scope is satisfied. Partial, unverified and ongoing entries
stay unstruck, even if a related component is implemented. A frozen-format item
requires the freeze decision; an implementation alone does not close it.
Item completion does not automatically close the containing stage or phase.

## 8. The checker

```sh
make check-docs                       # verify
make toc                              # regenerate TOCs and the ADR index
python3 tools/check-docs.py --journey # advisory journey-word counts
```

[`tools/check-docs.py`](../tools/check-docs.py) verifies relative links and `#anchors` (GitHub slug
rules, fenced code ignored); generates and verifies the TOC blocks; resolves
`ADR-NNN` and `MNN` mentions; generates and verifies the ADR index; requires
the navigation block; requires the index rows; and enforces the status rules
of section 3 (`Status:` lines only in ADRs, five README status rows, one-line
milestone status cells). It exits non-zero with one problem per line after a
single summary line. It ignores `crates/`, `native/`, `vendor/`, `target/`
and `build/`.

Document discovery prunes excluded trees before traversal, so concurrent Cargo
cleanup inside the build tree is not an input error. Errors in included
source directories are reported. Run `python3 tools/test-check-docs.py` for the
temporary-fixture discovery regressions.
