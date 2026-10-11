# Model library sources (FEATURES.md 68)

SketchUp's Window > 3D Warehouse opens Trimble's online model library. Meshtard's
equivalent is Window > Model Library (`window.3d_warehouse`, same toolbar button),
with pluggable sources. This page records which online libraries can be used by a
third-party client, and on what terms. Researched 2026-10-11.

## Trimble 3D Warehouse: not used

- No public API. SketchUp forum, "Integrate Warehouse using APIs" (2019): "there does
  not (yet) exist a public API for 3D Warehouse (although often requested!)"; the site
  has an internal JSON interface that is "not officially released for public usage".
  <https://forums.sketchup.com/t/integrate-warehouse-using-apis/98593>. A later
  thread says the same: the Warehouse is a separate web product without a public API,
  and the SketchUp C API only reads and writes local files.
  <https://forums.sketchup.com/t/importing-and-exporting-models-from-sketchup-warehouse/196220>
- Terms forbid the workaround. 3D Warehouse Terms of Use, section 10 (Conduct, Misuse):
  "(vi) Scraping information from 3D Warehouse" and "(xi) Taking any action that
  imposes an unreasonable or disproportionately large load on 3D Warehouse
  infrastructure"; the General Model License (section 2) also restricts
  "aggregating such content in another online warehouse or similar product or
  service". <https://embed-3dwarehouse-classic.sketchup.com/tos/> (classic-site copy,
  last updated 2017-04-01; the current terms may differ). FAQ:
  <https://help.sketchup.com/en/3d-warehouse/3d-warehouse-terms-use-faq>.
- Downloads also need a Trimble ID sign-in.
- Decision: not implemented. A user can still download a `.skp` from the 3D Warehouse
  website in a browser and place it from a Local folder (or File > Import). If Trimble
  publishes an API with terms that allow third-party clients, it can be one more source.

## Poly Haven: used (default online source)

- Public API, no key or account: <https://api.polyhaven.com>, docs and terms at
  <https://github.com/Poly-Haven/Public-API> (README, `ToS.md`).
- Terms (ToS.md): free for any use including commercial (2.1-2.3; "Poly Haven will
  never require payment, a license, or a key"); every call must carry "a unique
  Referer header or user-agent that matches your software name" (2.4); apps must make
  it clear to users where the content comes from with a lightweight credit line or
  logo, and must not imply endorsement (2.5); no activity that degrades the API (2.6);
  funding is optional (2.7). No rate limit numbers are published.
- Assets are CC0 (public domain), no attribution required.
- Meshtard: User-Agent `Meshtard/<version> (+https://github.com/noxqs/meshtard)`; the
  panel shows "Models from Poly Haven (CC0, free for any use). Not affiliated with Poly
  Haven." while the source is picked; API calls are spaced at least 250 ms apart and
  the full model list (`/assets?type=models`, about 1 MB, ~520 models) is cached for a
  day and searched locally; thumbnails come from the CDN as 256 px PNG and are cached;
  downloads take the 1k-texture glTF from `/files/{id}` (the `.gltf` plus its
  `include` files: `.bin` and textures).
- Other Poly Haven API users (Blender add-ons, game engine plugins) work the same way.

## Sketchfab: optional, with the user's own token

- Download API: <https://sketchfab.com/developers/download-api> and guidelines
  <https://sketchfab.com/developers/download-api/guidelines>. Search
  (`GET /v3/search?type=models&downloadable=true&q=`) needs no token;
  `GET /v3/models/{uid}/download` needs an authenticated user and returns short-lived
  URLs of glTF (zip), GLB and USDZ archives (not source formats).
- Guidelines: downloads require users to be authenticated with their Sketchfab account
  (apps are expected to offer an OAuth 2.0 login; downloading without end-user
  authentication needs an agreement with Sketchfab); apps must show each model's
  license and author; Creative Commons licenses require crediting the author and
  Sketchfab. Licenses vary per model (CC BY, BY-SA, BY-NC, BY-ND, BY-NC-ND, CC0, ...);
  some forbid commercial use or modification.
- Meshtard: the source appears only after the user enters their own API token
  (Sketchfab Settings > Password & API) in the panel; it is kept in plain text in
  `<config>/meshtard/library.toml` and sent as `Authorization: Token ...`. The GLB is
  downloaded. License label, license link and author are shown and stored on the
  definition. Left: an OAuth login (needs a registered Sketchfab app / client id), which
  the guidelines prefer over personal tokens.

## Other open libraries (not added yet)

- Smithsonian Open Access: thousands of CC0 3D models (glTF/OBJ) on 3d.si.edu; the Open
  Access API (api.si.edu) needs a free api.data.gov key; CC0 assets free for any use.
  <https://www.si.edu/openaccess>, <https://3d.si.edu/collections/openaccesshighlights>.
  A candidate for a later source.
- Kenney, Quaternius (CC0 game assets): downloads as packs, no search API.
- NASA 3D Resources: a GitHub repository, no search API, own media-usage guidelines.

## Network client

- `ureq` 3.4 (MIT/Apache-2.0), blocking, on background threads only; TLS by `rustls`
  (Apache-2.0/ISC/MIT) with the `ring` provider (Apache-2.0 AND ISC) and
  `rustls-platform-verifier` (system certificates; on Linux the system store or
  `SSL_CERT_FILE`); `webpki-roots` (CDLA-Permissive-2.0) comes along with the
  `rustls` feature. Proxies from `HTTPS_PROXY` / `ALL_PROXY`.
- Responses are parsed by pure functions (`library::polyhaven`, `library::sketchfab`)
  tested with small hand-made JSON; one ignored live test
  (`cargo test -p mt-app live_ -- --ignored`) lists Poly Haven, fetches a thumbnail and
  downloads and reads the Rubber Duck model.
