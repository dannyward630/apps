---
"@servers/git-tracker-api": patch
---

Reject NUL characters in upload and completion text with field-specific client errors before persistence or GitHub resolution. Preserve existing text syntax and numeric behavior.

<!-- public-changelog:start -->

Git Tracker now reports an invalid upload or completion field when its text contains a NUL character, instead of returning a server error. Remove the NUL character and retry the request.

<!-- public-changelog:end -->
