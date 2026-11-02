# Source font

These unmodified WOFF2 files come from the official
[JetBrains Mono v2.304 release](https://github.com/JetBrains/JetBrainsMono/releases/tag/v2.304),
under `fonts/webfonts/`. The original [SIL Open Font License](OFL.txt) accompanies
the fonts and is available from the embedded UI at `/fonts/OFL.txt`.

The `Rastok Source` CSS family uses these faces only in the source pane.
It supplies coding ligatures independently of installed fonts, without a
runtime download from an external service. The static assets are included only
with `hosted-ui`. Existing keyword weight and comment italic styling remain;
the browser synthesizes italics from the regular face.

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| JetBrainsMono-Regular.woff2 | 92164 | `a9cb1cd82332b23a47e3a1239d25d13c86d16c4220695e34b243effa999f45f2` |
| JetBrainsMono-SemiBold.woff2 | 94472 | `918edad542a1da608fd2ba8daebaff9ac802309103fe760eed465b8b4e47faf1` |
| OFL.txt | 4399 | `30f0c136e3c88e422d0791acd97238870f9054a9729bc34cf2ff0d4ed8cac4ad` |

There is no build tool or transitive dependency. Updates replace the pinned
assets and hashes after checking provenance and the license. Removing the font
requires removing its static routes and CSS faces; source positions and plan
formats do not depend on it.
