# Turninator desktop interface

The refactor keeps the paired inspection/render workspace and organizes settings around the work a user is doing. It preserves render identities, asynchronous import ownership, saved projects and the existing output formats.

| Task | Settings |
| --- | --- |
| Model | Orientation/down, stepped geometry, physical layer heights, source assumptions and advanced import settings |
| Appearance | Material, base/mapped colors, layer shading, roughness/resin and studio |
| View | Camera, mesh inspection, fixed printed stage and cutaway analysis |
| Export | Format, contextual codec/FPS/alpha, render quality and animation |

Preview and export actions remain visible below the scrolling settings. Each task retains its own scroll position. A model summary stays above navigation. Changing tasks does not modify the scene, camera, revision or retained-frame identity. RGB pickers commit only actual edits, avoiding incidental RGB/HSV rounding while painting a previously hidden tab.

Controls use consistent label-above fields, full-width selectors/numbers, unit suffixes, readable choices, larger mouse targets and the operating-system light/dark theme. Related XYZ values share a row. Advanced interpretation settings stay collapsed. Fixed sheets explain their fixed appearance and omit presentation controls. Existing MP4-only codec, animation-only framerate and PNG-only alpha rules remain in force, retaining hidden preferences.

File actions use a conventional menu and keyboard shortcuts: Ctrl+O opens a model, Ctrl+Shift+O opens a project, and Ctrl+S saves a project. Specific shortcuts take precedence; holding a key does not open repeated dialogs. Settings separates application tool paths and retained-frame storage from model configuration. Its contents scroll independently of the workspace. Errors, job progress, cancellation and output actions remain in the status area.

The animation-keyframe toggle is always visible on Export; active timeline controls appear beneath it. Simple turntables show speed, duration and resulting turns. Fixed-camera build-up explains how to enable camera movement. Source and section diagnostics remain available through disclosure controls rather than raw diagnostic data occupying the primary form.

Completed render images appear in the comparison panel as they become available, including animation frames, section views and four-view panels. A background loader keeps the latest pending image when rendering outpaces display. Scene edits, cancellation and replacement jobs invalidate late images. The last render image stays visible while the saved output loads, then the panel switches to the actual encoded output.

GIF playback decodes from disk on a background thread with at most three display frames queued. Pausing applies backpressure; looping rewinds the same open file and resets the GIF compositor. Playback retains encoded delays, transparency and disposal behavior without loading the entire animation into RAM. Display images keep the existing 1280-pixel maximum dimension; exported files remain at their requested resolution.

## Design guidance

- [Microsoft: Forms](https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/forms) recommends clear labels above fields, grouped related controls, responsive layouts and logical keyboard order. The native desktop controls use compact mouse-oriented sizing rather than copying the XAML examples' touch dimensions.
- [NN/g: Progressive Disclosure](https://www.nngroup.com/articles/progressive-disclosure/) supports exposing common choices first and revealing secondary settings when needed.
- [NN/g: Consistency and Standards](https://www.nngroup.com/articles/consistency-and-standards/) supports familiar file actions, predictable terminology and consistent control treatment.

These are design principles, not a claim of platform-style or accessibility certification. Actual painted-egui tests and optimized desktop screenshots cover control placement, contextual visibility, pointer navigation, shortcuts, retained preferences and narrow-window behavior. Verification and package evidence are recorded in [progress.md](progress.md).
