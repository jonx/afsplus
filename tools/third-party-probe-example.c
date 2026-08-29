/*
 * Pseudocode-style example of how a generic disk utility should identify AFS+.
 * Exact offsets and magic are not frozen until epoch 1.
 */

int probe_afsplus(int fd, struct probe_result *out)
{
    unsigned char buf[4096];

    if (pread(fd, buf, sizeof(buf), AFSP_IDENT_OFFSET) != sizeof(buf))
        return 0;

    if (!afsplus_ident_magic_matches(buf))
        return 0;

    if (!afsplus_ident_checksum_valid(buf))
        return 0;

    out->type = "afsplus";
    out->uuid = afsplus_ident_uuid(buf);
    out->label = afsplus_ident_label(buf);
    return 1;
}
