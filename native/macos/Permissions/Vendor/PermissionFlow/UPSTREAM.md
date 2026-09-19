> 🤖🔧 ai generated

Source: https://github.com/jaywcjlove/PermissionFlow
Revision: 2f2a4b76b1eb2ff7ab815b977be8229853f10bf8

Only PermissionFlow and SystemSettingsKit are vendored with the local adaptations listed below. Portal compiles these modules directly and bundles upstream localization resources under their expected bundle name. Retain upstream MIT license when distributing.

Portal exposes PermissionFlowPane.localizedTitle for conservative matching of the active privacy pane before returning from permission setup. Unknown or mismatched localized titles leave Settings open.

Portal adds a text-only settings guide to the existing floating panel controller, reusing window tracking, positioning, dismissal, and Settings navigation. Wi-Fi helper approval uses this mode because background activity is a toggle, not an app drag target.
