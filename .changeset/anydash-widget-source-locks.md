---
"@servers/anydash-api": patch
---

Hold source rows during widget validation and lock widgets before detaching their sources. This keeps deletion, widget removal, and sharing updates in a consistent order under concurrent requests.

<!-- public-changelog:start -->

Widgets stay consistent when you delete a connection or device while changing or removing a widget.

<!-- public-changelog:end -->
