---
name: spec-list
description: Show where the change specs under docs/specs stand, Implementing, Ready, Draft, Shelved and recently Done. Use when the user asks to list specs, see the ready work, the drafts, or the progress of one in flight.
---

# Spec list

From the repository's root, run the project's helper and pass on what it prints:

```sh
cargo run -q --no-default-features --example spec_list [bucket]
```

The user's argument, if any, is the bucket: `implementing`, `ready`, `draft`, `shelved` or
`done`. Without one, leave it out.

Show its output **as it is**: no summary, nothing rearranged, nothing before or after it. On an
error, show what it wrote to stderr and stop.

It has two other modes, for upkeep rather than browsing (`examples/spec_list.rs` says more):

- `-- --index` rewrites the Index in `docs/specs/README.md` from the specs' headers: number,
  title, status. Run it after a spec is written or its status or title changes.
- `-- --check` says whether they agree, as `tests/docs.rs` does.
