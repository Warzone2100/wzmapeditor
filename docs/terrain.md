# Terrain tab

The Terrain tab holds every map-editing brush along with the mirror
controls.

## Tools at a glance

| Tool | Description |
|------|-------------|
| Height brush | Sculpts terrain height with a soft falloff. Drag to paint. Modes: Raise, Lower, Smooth, Set. |
| Vertex sculpt | Click a vertex and drag to raise or lower it. `Ctrl+drag` box-selects. |
| Texture paint | Paints the selected tile, with optional rotation and flip. |
| Ground type | Paints a random tile from a weighted pool. Edit pools in the Tileset tab. |
| Stamp | Captures a tile and object pattern, then replays it on click. |
| Wall | Drag to place walls. Corners snap automatically. |

The Texture and Ground type tools share a button; the **Single** /
**Pool** toggle selects between them.

## Shared brush behaviour

- **`Ctrl+click`** eyedrops the tile, orientation, height, or ground
  type under the cursor into the active brush.
- **`Shift+click`** arms line-draw mode on the Texture, Ground type,
  and Height-brush Set tools. The next click stamps a straight line at
  the current brush size. `Esc`, right-click, or switching tools
  cancels.
- **Undo** treats one stroke — drag, line, or stamp — as a single
  step.

## Height brush

Four sub-modes share a single brush. Raise, Lower, and Smooth re-fire
while the mouse is held; Set commits once per tile.

- **Mode** (`1` – `4`) — Raise, Lower, Smooth, Set.
- **Radius** (0 – 20 tiles) — brush size; drawn as a circle under the
  cursor.
- **Strength** (0.1 – 5.0) — per-tick height delta in Raise / Lower /
  Smooth modes.
- **Target height** (0 – 510) — destination height for Set mode.

## Vertex sculpt

Click to select individual vertices, then drag to raise or lower them
with a soft falloff. Selections persist across clicks; vertices already
in the selection drag together.

- **Soft radius** (0 – 12 tiles) — falloff zone around each selected
  vertex.
- **Selection count** — how many vertices are currently held.
- **Clear selection** — drop all selected vertices.
- **`Shift+click`** adds to or removes from the selection.
- **`Ctrl+drag`** box-selects; **`Ctrl+Shift+drag`** adds to the
  existing selection.

## Texture paint (Single)

Paints one chosen tile per stroke. Choose the source tile from the
Tileset tab; rotation and flip come from the buttons below.

- **Radius** (0 – 20 tiles) — brush size.
- **Set texture** — write the tile index. Turn off to repaint only
  rotation / flip.
- **Set orientation** — write the rotation / flip. Turn off to repaint
  only the tile index.
- **Rotate ↺ / ↻ / Flip X** — quick orientation controls; the current
  rotation and flip are shown alongside.
- **Randomize** — pick a fresh random rotation and flip per tile.

## Ground type (Pool)

Paints a random tile drawn from the selected ground type pool. Each
stamped tile gets a fresh random rotation and flip.

- **Radius** (0 – 20 tiles) — brush size.
- **Active pool** — the ground type being sampled. Edit pools and
  their weights in the **Tileset** tab.
- **Pool info** — name and tile count for the active pool; warns when
  the pool is empty.

## Stamp

The Stamp tool captures a rectangular patch of map and replays it
elsewhere. It has two phases:

1. **Capture** — drag a rectangle to record the tiles, heights, and
   objects inside it. Right-click to discard and re-capture.
2. **Place** — switch to either mode below and apply the stamp. Right-
   click returns to the capture phase.

### Single

One click drops the full captured pattern, centred on the cursor. The
preview is green when the stamp fits on the map and red when it would
spill off the edge.

- **Stamp tiles** — write the captured tile textures and orientations.
- **Stamp terrain** — write the captured tile heights.
- **Stamp objects** — place the captured structures, droids, and
  features.
- **Random rotation** — pick a random 90° rotation per click.
- **Random flip** — pick a random X/Y flip per click.

### Scatter

Drag inside a circular brush to scatter randomly-sampled objects from
the captured pattern.

- **Radius** (1 – 20 tiles) — brush size.
- **Density** (0.01 – 1.0 per tile²) — objects per square tile; the
  panel shows the expected burst count.
- **Stroke spacing** (1 – 10 tiles) — minimum cursor travel between
  scatter bursts while dragging.
- **Min object spacing** (0 – 256 world units) — minimum gap between
  objects within a single burst.
- **Random rotation** and **Random flip** — as in Single mode.

## Wall

Drag to paint a connected run of walls. The tool inspects each tile's
four neighbours and picks the right straight / corner / T / cross
piece automatically.

- **Family** — base wall stat: Hardcrete Mk1, Collective, NEXUS, BaBa,
  Tank Trap, plus any modded walls.
- **Cross-shape corners** — when the family has a dedicated cross
  variant (`CWall`), save L-corners under that stat instead of the
  base. Disabled for families without a cross piece.

## Mirror

The mirror selector reflects every edit across one or both map axes:

- **Off** — no mirroring.
- **Vertical** — across the left/right midline.
- **Horizontal** — across the top/bottom midline.
- **Both** — 4-way reflection.
- **Diagonal** — both diagonals.

The active mirror axes are drawn in the viewport while a brush is
selected, and the eyedropper, line preview, and undo all respect them.

## Shortcuts

| Key | Action |
|-----|--------|
| `1` – `4` | Height brush mode (Raise / Lower / Smooth / Set) |
| `5` | Texture paint |
| `6` | Ground type |
| `7` | Stamp |
| `8` | Wall |
| `V` | Vertex sculpt |
| `R` | Rotate the active placement |
| `Ctrl+click` | Eyedropper |
| `Shift+click` | Line draw (Texture / Ground type / Height Set) |
| `Esc` | Cancel line draw, deselect, or return to Object Select |
| Right-click | Cancel line draw or stamp capture |
| `Ctrl+Z` / `Ctrl+Y` | Undo / Redo |
| `Shift+scroll` | Adjust camera move speed |
