# Unreleased

Changes since 0.1.0.

## Saving to .skp keeps more

- **The current view is saved.** A file opens where you left it (camera position, perspective or parallel projection, field of view), not in the Iso view. Two-point perspective is saved as plain perspective.
- **Arcs and circles stay arcs.** They reopen as arcs and circles you can edit by radius and segment count, not as plain curves. Convert to Polygon is not kept yet (the curve reopens as an arc).
- **Edge tags are saved.** Edges keep their own tag.
- **Guides and section planes inside groups and components are saved.**
- Writer library (`mt_skp::write`): `add_circle` / `add_arc` off the ground plane wrote a wrong second axis (inherited from OpenSKP's writer) that readers take for an ellipse; fixed.

Still not saved: scenes, shadow settings, fog, model axes, locks, the active section plane, two-point perspective, text and dimension tags, tag folders and dashes, environments, fonts and dimension expert settings. Where the SketchUp 2017 format keeps most of these is not known yet; files saved by SketchUp during the trial will show it (`spec/parity/TRIAL_CHECKLIST.md` 1b). Tag folders and environments came after SketchUp 2017 and need the newer file format.
