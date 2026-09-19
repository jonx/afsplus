#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-2-Clause
"""Every tool's --json output validates against its published schema.

    check-json-schemas.py [--bin <dir>] [--vectors <dir>]

The schemas are spec/schemas/<tool>-<version>.schema.json (JSON Schema
2020-12). This gate runs each tool on the probe kit's vectors
(tools/check-probe-kit.sh, default build/probe-kit/vectors) and validates
the answer with the validator below, which covers the subset the schemas
use: type, properties, required, additionalProperties, items, minItems,
maxItems, enum, const, pattern, minimum, oneOf. Nothing outside the
standard library. Exit 0 only when every answer validates, and the two
controls fail: a wrong schema_version, and an extra top-level key.
"""
import json, os, re, subprocess, sys, tempfile

repo = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
args = sys.argv[1:]
def opt(name, default):
    return args[args.index(name) + 1] if name in args else default
bin_dir = opt("--bin", os.path.join(os.environ.get("CARGO_TARGET_DIR", os.path.join(repo, "target")), "release"))
vectors = opt("--vectors", os.path.join(repo, "build", "probe-kit", "vectors"))
schemas_dir = os.path.join(repo, "spec", "schemas")

TYPES = {"object": dict, "array": list, "string": str, "boolean": bool, "null": type(None)}

def check(value, schema, path="$"):
    """Returns the first violation as a string, or None."""
    if "const" in schema and value != schema["const"]:
        return "%s: expected %r, got %r" % (path, schema["const"], value)
    if "enum" in schema and value not in schema["enum"]:
        return "%s: %r not in %r" % (path, value, schema["enum"])
    if "type" in schema:
        allowed = schema["type"] if isinstance(schema["type"], list) else [schema["type"]]
        ok = False
        for t in allowed:
            if t == "integer" and isinstance(value, int) and not isinstance(value, bool): ok = True
            elif t == "number" and isinstance(value, (int, float)) and not isinstance(value, bool): ok = True
            elif t in TYPES and isinstance(value, TYPES[t]) and not (t != "boolean" and isinstance(value, bool)): ok = True
        if not ok:
            return "%s: type %s, got %s" % (path, allowed, type(value).__name__)
    if "oneOf" in schema:
        matches = [i for i, s in enumerate(schema["oneOf"]) if check(value, s, path) is None]
        if len(matches) != 1:
            return "%s: matches %d of %d alternatives" % (path, len(matches), len(schema["oneOf"]))
    if isinstance(value, dict):
        props = schema.get("properties", {})
        for k in schema.get("required", []):
            if k not in value:
                return "%s: missing %r" % (path, k)
        for k, v in value.items():
            if k in props:
                err = check(v, props[k], "%s.%s" % (path, k))
                if err: return err
            elif schema.get("additionalProperties") is False:
                return "%s: unexpected key %r" % (path, k)
    if isinstance(value, list):
        if "minItems" in schema and len(value) < schema["minItems"]:
            return "%s: fewer than %d items" % (path, schema["minItems"])
        if "maxItems" in schema and len(value) > schema["maxItems"]:
            return "%s: more than %d items" % (path, schema["maxItems"])
        if "items" in schema:
            for i, v in enumerate(value):
                err = check(v, schema["items"], "%s[%d]" % (path, i))
                if err: return err
    if isinstance(value, str) and "pattern" in schema and not re.search(schema["pattern"], value):
        return "%s: %r does not match %s" % (path, value, schema["pattern"])
    if isinstance(value, int) and not isinstance(value, bool) and "minimum" in schema and value < schema["minimum"]:
        return "%s: %r below %r" % (path, value, schema["minimum"])
    return None

def schema(name):
    return json.load(open(os.path.join(schemas_dir, name + ".schema.json")))

def run(cmd):
    r = subprocess.run(cmd, capture_output=True, text=True)
    out = r.stdout.strip()
    try:
        return json.loads(out), r.returncode
    except ValueError:
        return None, r.returncode

checks = fails = 0
def ok(cond, what):
    global checks, fails
    checks += 1
    if not cond:
        fails += 1
        print("  FAIL " + what)

def tool(name):
    return os.path.join(bin_dir, name)

v = lambda n: os.path.join(vectors, n)
for needed in ("empty.afsp", "one-file.afsp", "sparse.afsp", "links.afsp", "corrupt-object-checksum.afsp"):
    if not os.path.exists(v(needed)):
        print("check-json-schemas: missing %s; run tools/check-probe-kit.sh first" % v(needed)); sys.exit(69)

with tempfile.TemporaryDirectory() as tmp:
    doc, rc = run([tool("mkafsplus"), "--json", "--size-mib", "8", "--label", "Schema", os.path.join(tmp, "s.afsp")])
    ok(rc == 0 and check(doc, schema("mkafsplus-1")) is None, "mkafsplus: %s" % check(doc, schema("mkafsplus-1")))
    for img in ("empty", "one-file", "sparse", "links", "large-dir"):
        for t, s in (("afsplus-info", "afsplus-info-1"), ("afsplus-dump", "afsplus-dump-1")):
            doc, rc = run([tool(t), v(img + ".afsp"), "--json"])
            err = check(doc, schema(s)) if doc is not None else "no JSON"
            ok(rc == 0 and err is None, "%s on %s: %s" % (t, img, err))
        doc, rc = run([tool("afsplus-check"), v(img + ".afsp"), "--json"])
        err = check(doc, schema("afsplus-check-5")) if doc is not None else "no JSON"
        ok(err is None and doc["clean"] is True, "afsplus-check on %s: %s" % (img, err))
    doc, rc = run([tool("afsplus-check"), v("corrupt-object-checksum.afsp"), "--json"])
    err = check(doc, schema("afsplus-check-5")) if doc is not None else "no JSON"
    ok(err is None and doc["clean"] is False and doc["errors"], "afsplus-check on a damaged volume: %s" % err)
    for question in (["block", "0"], ["object", "1"], ["path", "/hello.txt"], ["checkpoint"], ["reclaim"], ["space", "0"], ["feature"]):
        doc, rc = run([tool("afsplus-explain"), v("one-file.afsp"), "--json"] + question)
        err = check(doc, schema("afsplus-explain-2")) if doc is not None else "no JSON"
        # `feature` alone answers the whole registry, kind `features`.
        ok(err is None and doc["kind"] in (question[0], question[0] + "s"), "afsplus-explain %s: %s" % (" ".join(question), err))
    doc, rc = run([tool("afsplus-image-diff"), v("empty.afsp"), v("one-file.afsp"), "--json"])
    err = check(doc, schema("afsplus-image-diff-3")) if doc is not None else "no JSON"
    ok(rc == 0 and err is None, "afsplus-image-diff: %s" % err)
    probe = os.path.join(repo, "build", "probe-kit", "afsplus-probe")
    for img, expect in (("one-file.afsp", True), ("foreign.img", False), ("damaged.afsp", False)):
        doc, rc = run([probe, "--json", v(img)])
        err = check(doc, schema("afsplus-probe-1")) if doc is not None else "no JSON"
        ok(err is None and doc["afsplus"] is expect, "afsplus-probe on %s: %s" % (img, err))

    # Controls: the validator must refuse a wrong version and an extra key.
    doc, _ = run([tool("afsplus-info"), v("empty.afsp"), "--json"])
    wrong = dict(doc, schema_version=99)
    ok(check(wrong, schema("afsplus-info-1")) is not None, "control: a wrong schema_version is refused")
    extra = dict(doc, surprise=1)
    ok(check(extra, schema("afsplus-info-1")) is not None, "control: an extra top-level key is refused")

print("json-schemas: %d checks, %d failures" % (checks, fails))
sys.exit(1 if fails else 0)
