# PAX completion envelope version 2

[ADR-081](../adr/ADR-081-ordinary-pax-completion-member.md) defines the integrity and
termination contract. [Archive qualification](../testing/backup-archive-qualification.md)
owns its executable gates. This envelope carries a body whose preservation
semantics are validated separately.

## Control headers

The first tar member is a PAX global header named
`_AROS_BACKUP/begin`. Its unique UTF-8 records are exactly:

| Keyword | Value |
|---|---|
| `AROS.backup.envelope` | `2` |
| `AROS.backup.algorithm` | `sha512-256` |

The final member is an ordinary regular file named
`_AROS_BACKUP/complete.pax`. Its unique records are exactly:

| Keyword | Value |
|---|---|
| `AROS.backup.end` | `2` |
| `AROS.backup.bytes` | Decimal byte count immediately before this header |
| `AROS.backup.members` | Decimal body-header count |
| `AROS.backup.hash` | 64 lowercase hexadecimal characters encoding SHA-512/256 |

Each control header uses the exact path and type above, zero uid/gid/mtime,
empty link/user/group strings, and its actual PAX payload byte size. The
beginning mode is zero; the terminal mode is `0600`. Payloads
are at most 4096 bytes, with standard zero block padding. Unknown or duplicate
fields are rejected. Record order is immaterial. Decimal counts use canonical
unsigned syntax, with `0` as the sole leading-zero form. Repeating the beginning
control inside the body is invalid. Body global headers are refused. Source objects occupy `files/`, with a `files`
directory representing the root. Auxiliary records occupy `_AROS_BACKUP/metadata/`.
Body paths are canonical relative paths without empty, dot or parent components;
only directories may have a trailing slash. Hard-link targets stay under `files/`.
Local PAX headers carry metadata for interpretation by the preservation layer,
which must validate resolved names against the same namespace rules. A symlink
target is data and never grants authority to traverse it during restoration.
Profile-aware restoration consumes control/auxiliary entries separately and
unwraps the source root; ordinary tar recovery exposes the `files` subtree.

## Digest domain and completion

The digest begins at byte zero and ends immediately before the final control
header. It covers complete beginning/body headers, payloads and padding. Its
byte count uses unsigned 128-bit arithmetic and cannot exceed `(2^128-1)/8`,
keeping the input below the hash standard's bit-length limit. The streamed
archive, including control records and end framing, fits the same bound. Body member counts
include every body tar header, including PAX metadata headers, and exclude the
two envelope control headers. Counts fit unsigned 64-bit framing admission.

A valid terminal record is followed by two zero tar end blocks and EOF. A caller
may admit a bounded number of additional complete zero blocks. Any later member,
partial trailing block or nonzero trailing bytes cause failure. Integrity receipts
are available only after counts, hash, end framing and EOF all verify. Output
receipts require successful end-marker output and sink flush; the sink defines
whether flush implies durable storage.

Control parsing uses fixed limits: 4096 payload bytes, four records, 64 keyword
bytes and 128 value bytes. Body member/payload limits are supplied independently.
Stream payloads through caller buffers. No allocation may scale with a declared
file size. Higher layers validate PAX size overrides before applying them to
framing. No override is permitted for an envelope control header.

The unkeyed hash detects changed or incomplete streams without authenticating a
sender. Full preservation or content recovery requires its own profile validation,
source/restore authority and explicit loss reporting under
[ADR-078](../adr/ADR-078-backup-preservation-modes.md).

## Effective ordinary member fields

The ordinary-member admission interface resolves a single local PAX block
before payload-size selection. It accepts `path`, `linkpath`, `size`, `uid`,
`gid`, `mtime`, `uname` and `gname`. Other keywords require a dedicated
preservation-profile handler and cause explicit refusal at this interface.
The explicit [sparse-aware interface](backup-sparse.md) admits GNU sparse 1.0
with separate logical and stored sizes. Default constructors refuse sparse,
attribute and security transports; ignoring their records is not content recovery. Duplicate keywords and configured record/byte budgets
are checked even for directly supplied records.

Resolved paths obey the body namespace rules above. Hard-link targets obey the
source namespace rules. Symlink targets are nonempty data, with no traversal
authority. Ordinary files and directories have empty link fields; every non-file
member has zero effective payload size. PAX header members themselves are not
ordinary objects. Strings are borrowed from admitted inputs, bounded by the
configured value-byte limit and free of NUL bytes.

The interface retains numeric identities and textual owner/group names as
separate archive facts. It performs no host account lookup or authorization
mapping; that policy belongs to the preservation consumer.

Size and identity overrides use canonical unsigned decimal values through
`2^64-1`. Timestamps use signed 64-bit floor seconds and nanoseconds below
`10^9`. Decimal input has an optional minus sign, canonical whole digits and
an optional fraction of one through nine digits. Negative zero, excessive
precision and overflow are refused without rounding. For example, `-1.25`
means seconds `-2` plus `750000000` nanoseconds. Output removes trailing
fractional zeros. Negative timestamp interchange is an explicit profile
extension; ordinary tools may have narrower timestamp support.

This interface implements the resolved-field checks required by
[ADR-081](../adr/ADR-081-ordinary-pax-completion-member.md); it does not establish
archive-wide object identity, metadata completeness, or restore completion.

## Streaming local-record binding

The ordinary-member stream binds at most one local PAX block to the immediately
following ordinary header. Stacked blocks and a block followed by envelope
completion are invalid. Metadata bytes are admitted before allocating the
record buffer. All effective fields are validated before selecting payload
framing or exposing the member to the consumer. Overrides expire when advancing
to the next member.

The caller supplies payload buffers and must finish the current payload before
advancing. An early advance reports busy without consuming input. Malformed
metadata, framing and uncertain input errors permanently fail the stream;
completion cannot be recovered by skipping the failed member. Empty or finished
payload reads return zero. An integrity receipt is exposed only after ordinary
member admission, envelope verification and actual EOF, with no pending local
records. It does not certify preservation completeness or successful restoration.


## Raw size extension

Under [ADR-088](../adr/ADR-088-sparse-stored-size-field.md), the raw 12-byte
size field supports checked positive GNU binary encoding beyond the 33-bit
octal limit. Other numeric fields retain octal encoding. Negative/overflowing
binary sizes are invalid. Ordinary PAX size overrides retain their admission
contract; sparse members instead carry stored size only in the raw header.
See [sparse header admission](backup-sparse.md#header-admission).
