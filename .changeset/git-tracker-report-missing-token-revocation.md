---
"@servers/git-tracker-api": patch
---

Return not-found without a revocation audit row when a token ID is absent from the requested repository. Parse token IDs as UUIDs and keep revocation of an existing token idempotent.

<!-- public-changelog:start -->

Revoking a token now reports an error if the token does not belong to the selected repository, so a mistaken token ID no longer appears to be revoked.

<!-- public-changelog:end -->
