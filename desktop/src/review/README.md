# Desktop UI review

Run `npm run ui:review` from `desktop`, then open <http://127.0.0.1:1421/review.html>.

The review renders the actual desktop `App` in an iframe. Previous/next, the screen picker, searchable catalogue, window-size presets, reset, and temporary theme picker sit outside the app. The theme picker stays accessible during modal flows and preserves the current screen. The in-app appearance picker and external picker stay synchronized.

The URL stores every review preference: `state` (fixture ID), `view` (fit, desktop, compact, wide, narrow, or compare), `theme` (light, dark, or system), `sidebar` (1 or 0), and `filter` (catalogue search). Changes from controls, keyboard shortcuts, and the in-app theme picker update the same URL without adding history entries. Reloads, HMR, and shared links restore those preferences; Back/Forward restores them too. The filter stays saved when the catalogue is closed. Preview readiness and coverage checks are recomputed after a reload.

Choose **Desktop + narrow** in the size picker to compare 820 × 680 and 390 × 700 viewports side by side, or open `review.html?state=empty&view=compare`. Both show the selected fixture and share the theme picker and reset; each preview remains independently interactive. They scale visually to fit the available space while retaining their actual viewport dimensions. **Check all** verifies both previews in this mode.

Keyboard shortcuts work in the review shell and inside either preview: **← / →** go to the previous/next screen, **↑ / ↓** cycle through preview layouts, and **.** toggles light/dark (using the current resolved theme when System is selected). Screen navigation stops at the first/last screen; layouts wrap. Shortcuts leave text entry, native selects, radios, dropdowns, and other arrow-operated controls alone, and pause during **Check all**.

`scenarios.ts` lists screens, fixture IDs, interaction steps, and checkpoints. `core.ts` provides local, in-memory native responses. Pending requests stay pending, errors stay reproducible, and reset recreates a fixture. Fixtures use synthetic names, identities, account data, addresses, codes, and USB descriptors. Account links and native operations do not leave the preview.

**Check all** walks every fixture and checks that its expected screen opens, reporting failures. It checks rendering and flow entry, not real device, network, OS-permission, or server behavior. Run it in either theme. The narrow window preset tests the desktop layout at a small width; it is not the Android app.

Native aliases apply only when serving Vite in `ui-review` mode. Normal development uses port 1420, and the production entry and build do not import review files. Remove the `ui:review` script, review HTML files, this directory, and review aliases to discard the temporary tooling.
