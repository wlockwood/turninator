// Studio renderer: three.js raster scene in Z-up millimetres. Owns the
// object hierarchy  turntable -> display -> mesh(print matrix)  so that
// layers follow the build frame while presentation poses and turntable
// rotation change only the display.

import * as THREE from 'three';
import { OrbitControls } from 'three/addons/controls/OrbitControls.js';
import { createStudioEnvironment } from './studioEnv';
import { EffectComposer } from 'three/addons/postprocessing/EffectComposer.js';
import { RenderPass } from 'three/addons/postprocessing/RenderPass.js';
import { GTAOPass } from 'three/addons/postprocessing/GTAOPass.js';
import { OutputPass } from 'three/addons/postprocessing/OutputPass.js';
import { applyLayerShader, createLayerUniforms, type LayerUniforms } from './layerShader';
import { Inspector, type InspectionSettings } from './inspect';
import type { BeadChunkGeometry } from './pathtrace';
import { cutBelow } from '../core/cut';
import { addLayerTexturing, createLayerNormalMap, type LayerTexParams } from './layerTexturing';
import { applyBeadShader, createBeadGeometry, createBeadUniforms, type BeadColorMode, type BeadData, type BeadUniforms } from './beads';
import { directionFromAngles, envelopePoints, fitPerspective, type Vec3 } from './framing';
import type { LayerSettings, MaterialSettings, PoseSettings, StudioSettings } from '../state/settings';
import { orthoExtents } from './framing';
import { PANEL_ORDER, type PanelView } from '../export/fourview';

export interface FourViewRenderOptions {
  imageW: number;
  imageH: number;
  supersample: number;
  material: MaterialSettings;
  layers: LayerSettings;
  background: string;
  margin: number;
}

export interface FourViewPanels {
  shaded: Uint8ClampedArray[];
  normalDepth: Float32Array[];
  mmPerPx: number;
  sizeMm: [number, number, number];
}

const PANEL_DIRS: Record<PanelView, { dir: Vec3; up: Vec3 }> = {
  front: { dir: [0, 1, 0], up: [0, 0, 1] },
  right: { dir: [-1, 0, 0], up: [0, 0, 1] },
  top: { dir: [0, 0, -1], up: [0, 1, 0] },
  iso: { dir: [-1, 1, -1], up: [0, 0, 1] },
};

// Built from three's standard chunks so instanced bead shaders can inject into it.
const normalDepthMaterial = new THREE.ShaderMaterial({
  side: THREE.DoubleSide,
  clipping: true,
  vertexShader: /* glsl */ `
    #include <common>
    #include <clipping_planes_pars_vertex>
    varying vec3 vN;
    varying float vDepth;
    void main() {
      #include <beginnormal_vertex>
      #include <begin_vertex>
      #include <project_vertex>
      #include <clipping_planes_vertex>
      vN = normalize(normalMatrix * objectNormal);
      vDepth = -mvPosition.z;
    }`,
  fragmentShader: /* glsl */ `
    varying vec3 vN;
    varying float vDepth;
    #include <clipping_planes_pars_fragment>
    void main() {
      #include <clipping_planes_fragment>
      vec3 n = normalize(vN) * (gl_FrontFacing ? 1.0 : -1.0);
      gl_FragColor = vec4(n, vDepth);
    }`,
});
const beadNormalDepthMaterial = normalDepthMaterial.clone();


export type ViewName = 'front' | 'right' | 'top' | 'iso' | 'hero';

export interface Envelope {
  radius: number;
  height: number;
  size: [number, number, number];
}

const DEG = Math.PI / 180;
/** Experimental: bake Shaded layer ribs into a normal map for the path tracer. */
const PT_BAKED_RIBS = false;
/** Max supersampled tile edge rendered into the canvas at once. */
const TILE_PX = 4096;

function eulerMatrix(deg: [number, number, number]): THREE.Matrix4 {
  return new THREE.Matrix4().makeRotationFromEuler(new THREE.Euler(deg[0] * DEG, deg[1] * DEG, deg[2] * DEG, 'XYZ'));
}

/** Seamless sweep backdrop: floor (z=0) curving up into a wall behind +Y. */
export function sweepGeometry(width: number, front: number, radius: number, height: number): THREE.BufferGeometry {
  const profile: [number, number][] = [[-front, 0], [0, 0]];
  const steps = 24;
  for (let i = 1; i <= steps; i++) {
    const a = (i / steps) * (Math.PI / 2);
    profile.push([Math.sin(a) * radius, radius - Math.cos(a) * radius]);
  }
  profile.push([radius, height]);
  const pos: number[] = [];
  const idx: number[] = [];
  profile.forEach(([y, z]) => pos.push(-width / 2, y, z, width / 2, y, z));
  for (let i = 0; i < profile.length - 1; i++) {
    const a = i * 2, b = a + 1, c = a + 2, d = a + 3;
    idx.push(a, b, d, a, d, c);
  }
  const g = new THREE.BufferGeometry();
  g.setAttribute('position', new THREE.Float32BufferAttribute(pos, 3));
  g.setIndex(idx);
  g.computeVertexNormals();
  return g;
}

export class Studio {
  readonly renderer: THREE.WebGLRenderer;
  readonly scene = new THREE.Scene();
  readonly camera: THREE.PerspectiveCamera;
  readonly ortho: THREE.OrthographicCamera;
  readonly controls: OrbitControls;
  readonly turntable = new THREE.Group();
  readonly display = new THREE.Group();
  readonly material: THREE.MeshPhysicalMaterial;
  readonly layerUniforms: LayerUniforms = createLayerUniforms();
  private mesh: THREE.Mesh | null = null;
  readonly inspector: Inspector;
  private welded: Float32Array | null = null;
  private backdrop: THREE.Mesh;
  private backdropMat: THREE.MeshStandardMaterial;
  private key: THREE.DirectionalLight;
  private fill: THREE.DirectionalLight;
  private rim: THREE.DirectionalLight;
  private envTexture: THREE.Texture;
  private dirty = true;
  private raf = 0;
  private useOrtho = false;
  envelope: Envelope = { radius: 10, height: 10, size: [10, 10, 10] };
  /** Build-frame extents (mm): X and Y centered on 0, Z from 0. */
  buildSize: [number, number, number] = [10, 10, 10];
  onRender: (() => void) | null = null;

  constructor(private canvas: HTMLCanvasElement) {
    this.renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: true, stencil: true, preserveDrawingBuffer: false, powerPreference: 'high-performance' });
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    this.renderer.toneMapping = THREE.AgXToneMapping;
    this.renderer.toneMappingExposure = 1;
    this.renderer.outputColorSpace = THREE.SRGBColorSpace;
    this.renderer.localClippingEnabled = true;
    this.renderer.shadowMap.enabled = true;
    // PCF (not VSM): VSM's variance maths produced NaN/black blotches on thin walls.
    this.renderer.shadowMap.type = THREE.PCFShadowMap;

    THREE.Object3D.DEFAULT_UP.set(0, 0, 1);
    this.camera = new THREE.PerspectiveCamera(30, 1, 0.1, 10000);
    this.camera.up.set(0, 0, 1);
    this.ortho = new THREE.OrthographicCamera(-1, 1, 1, -1, 0.1, 10000);
    this.ortho.up.set(0, 0, 1);

    // Same procedural softbox studio as the path tracer, so both engines match.
    const pmrem = new THREE.PMREMGenerator(this.renderer);
    const equirect = createStudioEnvironment({ width: 512, height: 256 });
    this.envTexture = pmrem.fromEquirectangular(equirect).texture;
    equirect.dispose();
    pmrem.dispose();
    this.scene.environment = this.envTexture;
    this.scene.environmentRotation.set(Math.PI / 2, 0, 0);

    this.material = new THREE.MeshPhysicalMaterial({ color: 0x3d7be0, roughness: 0.6, side: THREE.DoubleSide });
    applyLayerShader(this.material, this.layerUniforms);

    this.backdropMat = new THREE.MeshStandardMaterial({ color: 0xe9e7e3, roughness: 1, metalness: 0 });
    this.backdrop = new THREE.Mesh(new THREE.BufferGeometry(), this.backdropMat);
    this.backdrop.receiveShadow = true;
    this.scene.add(this.backdrop);

    this.key = new THREE.DirectionalLight(0xffffff, 2.4);
    this.key.castShadow = true;
    this.key.shadow.mapSize.set(4096, 4096);
    this.key.shadow.bias = -0.0002;
    this.key.shadow.radius = 3;
    this.fill = new THREE.DirectionalLight(0xdfe8ff, 0.8);
    this.rim = new THREE.DirectionalLight(0xffffff, 1.6);
    this.scene.add(this.key, this.key.target, this.fill, this.fill.target, this.rim, this.rim.target);

    this.display.matrixAutoUpdate = false;
    this.turntable.add(this.display);
    this.scene.add(this.turntable);
    this.inspector = new Inspector(this.display, this.material);
    this.scene.add(this.inspector.capMesh);

    this.controls = new OrbitControls(this.camera, canvas);
    this.controls.enableDamping = true;
    // Short inertia: the view settles quickly instead of drifting (and
    // re-noising screen-space effects) for a second after release.
    this.controls.dampingFactor = 0.18;
    this.controls.addEventListener('change', () => this.invalidate('camera'));

    this.layoutStage();
    this.loop();
  }

  /** Listener for changes, used by the path-traced preview. */
  onChange: ((reason: 'scene' | 'camera' | 'transform') => void) | null = null;

  invalidate(reason: 'scene' | 'camera' | 'transform' = 'scene') {
    this.dirty = true;
    this.onChange?.(reason);
  }

  private loop = () => {
    this.raf = requestAnimationFrame(this.loop);
    this.controls.update();
    if (!this.dirty) return;
    this.dirty = false;
    this.resizeToCanvas();
    this.draw(this.activeCamera());
    this.onRender?.();
  };

  activeCamera(): THREE.Camera {
    return this.useOrtho ? this.ortho : this.camera;
  }

  private resizeToCanvas() {
    const w = this.canvas.clientWidth, h = this.canvas.clientHeight;
    if (w === 0 || h === 0) return;
    const pr = this.renderer.getPixelRatio();
    if (this.canvas.width !== Math.floor(w * pr) || this.canvas.height !== Math.floor(h * pr)) {
      this.renderer.setSize(w, h, false);
    }
    this.camera.aspect = w / h;
    this.camera.updateProjectionMatrix();
    this.updateOrthoAspect(w / h);
  }

  private updateOrthoAspect(aspect: number) {
    const halfH = (this.ortho.top - this.ortho.bottom) / 2;
    this.ortho.left = -halfH * aspect;
    this.ortho.right = halfH * aspect;
    this.ortho.updateProjectionMatrix();
  }

  dispose() {
    cancelAnimationFrame(this.raf);
    this.controls.dispose();
    this.renderer.dispose();
  }

  // ---------------------------------------------------------------- model

  private original: { positions: Float32Array; normals: Float32Array } | null = null;

  private raycaster = new THREE.Raycaster();

  /** Geometry-space normal of the model face under a client point, or null. */
  pickNormal(clientX: number, clientY: number): THREE.Vector3 | null {
    if (!this.mesh) return null;
    const rect = this.canvas.getBoundingClientRect();
    const ndc = new THREE.Vector2(((clientX - rect.left) / rect.width) * 2 - 1, -((clientY - rect.top) / rect.height) * 2 + 1);
    this.scene.updateMatrixWorld(true);
    this.raycaster.setFromCamera(ndc, this.activeCamera());
    const hit = this.raycaster.intersectObject(this.mesh, false)[0];
    if (!hit?.face) return null;
    const n = hit.face.normal.clone();
    // Face toward the viewer (DoubleSide meshes can report the back side).
    const toCam = this.raycaster.ray.direction.clone().negate();
    const world = n.clone().transformDirection(this.mesh.matrixWorld);
    if (world.dot(toCam) < 0) n.negate();
    return n;
  }

  /** World matrix of the model (print pose, display pose, turntable). */
  modelWorldMatrix(): THREE.Matrix4 {
    this.scene.updateMatrixWorld(true);
    return this.mesh ? this.mesh.matrixWorld.clone() : new THREE.Matrix4();
  }

  /** Everything the path tracer needs to mirror this scene. */
  pathTraceInput(transparent = false, beads?: { chunks: BeadChunkGeometry[]; vertexColors: boolean }) {
    if (!this.mesh || !this.lastMaterial || !this.lastStudio) throw new Error('No model loaded');
    if (this.modelKind === 'beads' && !beads) throw new Error('G-code must be baked before path tracing');
    this.scene.updateMatrixWorld(true);
    let geometry = this.modelKind === 'mesh' && this.buildProgress !== null ? this.cutGeometry(this.buildProgress) : this.mesh.geometry;
    let layerNormalMap: THREE.Texture | undefined;
    const l = this.lastLayers;
    // Baked normal-map ribs are disabled: three-gpu-pathtracer barely applies
    // this normal map (a constant 90° tilt changed brightness < 3%; see
    // PROGRESS.md). Kept behind a flag for a future fix; Sliced mode gives real
    // geometric layers in the path tracer.
    if (PT_BAKED_RIBS && this.modelKind === 'mesh' && l?.enabled) {
      // Bake shader-only layer ribs into UVs/tangents + a normal map for the path tracer.
      const params: LayerTexParams = {
        heightMm: l.heightMm, firstMm: l.firstMm, depthMm: l.depthMm, roundness: l.roundness,
        lineWidthMm: l.lineWidthMm, topLines: l.topLines, boundaries: this.boundaries,
      };
      const key = `${geometry.uuid}:${JSON.stringify({ ...params, boundaries: this.boundaries?.length ?? 0 })}:${this.mesh.matrix.elements.join(',')}`;
      if (this.layerTexCache?.key !== key) {
        this.layerTexCache?.geometry.dispose();
        this.layerTexCache?.map.dispose();
        this.layerTexCache = { key, geometry: addLayerTexturing(geometry, this.mesh.matrix, params), map: createLayerNormalMap(params) };
      }
      geometry = this.layerTexCache.geometry;
      layerNormalMap = this.layerTexCache.map;
    }
    return {
      geometry,
      layerNormalMap,
      modelMatrix: this.mesh.matrixWorld.clone(),
      backdropGeometry: this.backdrop.geometry,
      backdropMatrix: this.backdrop.matrixWorld.clone(),
      backdropVisible: !transparent,
      background: transparent ? null : this.lastStudio.background,
      material: this.lastMaterial,
      envIntensity: this.lastStudio.keyIntensity / 2.4,
      chunks: beads?.chunks,
      vertexColors: beads?.vertexColors,
    };
  }

  private cutCache: { key: string; geometry: THREE.BufferGeometry } | null = null;
  private layerTexCache: { key: string; geometry: THREE.BufferGeometry; map: THREE.Texture } | null = null;
  private boundaries: number[] | null = null;

  /** Model-space geometry cut at a build fraction and capped (path tracer build-up). */
  cutGeometry(fraction: number): THREE.BufferGeometry {
    const mesh = this.mesh!;
    const key = `${mesh.geometry.uuid}:${fraction.toFixed(5)}:${mesh.matrix.elements.join(',')}`;
    if (this.cutCache?.key === key) return this.cutCache.geometry;
    const pos = mesh.geometry.getAttribute('position') as THREE.BufferAttribute;
    const nrm = mesh.geometry.getAttribute('normal') as THREE.BufferAttribute;
    // Into the build frame, cut, and back to model space.
    const m = mesh.matrix;
    const rot = new THREE.Matrix3().setFromMatrix4(m);
    const bp = new Float32Array(pos.count * 3), bn = new Float32Array(pos.count * 3);
    const v = new THREE.Vector3();
    for (let i = 0; i < pos.count; i++) {
      v.fromBufferAttribute(pos, i).applyMatrix4(m);
      bp[i * 3] = v.x; bp[i * 3 + 1] = v.y; bp[i * 3 + 2] = v.z;
      v.fromBufferAttribute(nrm, i).applyMatrix3(rot);
      bn[i * 3] = v.x; bn[i * 3 + 1] = v.y; bn[i * 3 + 2] = v.z;
    }
    const cut = cutBelow(bp, bn, Math.max(1e-4, fraction) * this.buildSize[2]);
    const inv = m.clone().invert();
    const invRot = new THREE.Matrix3().setFromMatrix4(inv);
    for (let i = 0; i < cut.positions.length; i += 3) {
      v.set(cut.positions[i], cut.positions[i + 1], cut.positions[i + 2]).applyMatrix4(inv);
      cut.positions[i] = v.x; cut.positions[i + 1] = v.y; cut.positions[i + 2] = v.z;
      v.set(cut.normals[i], cut.normals[i + 1], cut.normals[i + 2]).applyMatrix3(invRot);
      cut.normals[i] = v.x; cut.normals[i + 1] = v.y; cut.normals[i + 2] = v.z;
    }
    const g = new THREE.BufferGeometry();
    g.setAttribute('position', new THREE.BufferAttribute(cut.positions, 3));
    g.setAttribute('normal', new THREE.BufferAttribute(cut.normals, 3));
    g.computeBoundingSphere();
    this.cutCache?.geometry.dispose();
    this.cutCache = { key, geometry: g };
    return g;
  }

  /** Print-pose matrix (geometry -> build frame), column-major. */
  get printMatrix(): number[] {
    return this.mesh ? [...this.mesh.matrix.elements] : new THREE.Matrix4().toArray();
  }

  /** Swap the rendered geometry (e.g. sliced mesh) without changing the pose basis. */
  setRenderGeometry(positions: Float32Array, normals: Float32Array) {
    if (!this.mesh || this.modelKind !== 'mesh') return;
    const g = new THREE.BufferGeometry();
    g.setAttribute('position', new THREE.BufferAttribute(positions, 3));
    g.setAttribute('normal', new THREE.BufferAttribute(normals, 3));
    g.computeBoundingSphere();
    this.mesh.geometry.dispose();
    this.mesh.geometry = g;
    this.inspector.setGeometry(g);
    this.invalidate();
  }

  /** Return to the imported mesh (with current smoothing normals). */
  restoreOriginalGeometry() {
    if (this.original) this.setRenderGeometry(this.original.positions, this.original.normals);
  }

  setModel(positions: Float32Array, normals: Float32Array, welded: Float32Array) {
    this.original = { positions, normals };
    this.modelKind = 'mesh';
    this.beadCount = 0;
    if (this.mesh) {
      this.display.remove(this.mesh);
      this.mesh.geometry.dispose();
    }
    const g = new THREE.BufferGeometry();
    g.setAttribute('position', new THREE.BufferAttribute(positions, 3));
    g.setAttribute('normal', new THREE.BufferAttribute(normals, 3));
    g.computeBoundingSphere();
    const mesh = new THREE.Mesh(g, this.material);
    mesh.renderOrder = 6; // after section-cap stencil passes
    mesh.castShadow = true;
    mesh.receiveShadow = true;
    mesh.matrixAutoUpdate = false;
    const m4 = new THREE.Matrix4();
    mesh.onBeforeRender = (_r, _s, cam) => {
      // Build frame (display-local) -> view space, rotation only.
      m4.multiplyMatrices(cam.matrixWorldInverse, this.display.matrixWorld);
      this.layerUniforms.uBuildToView.value.setFromMatrix4(m4);
    };
    this.mesh = mesh;
    this.welded = welded;
    this.inspector.setGeometry(g);
    this.display.add(mesh);
    this.invalidate();
  }

  setNormals(normals: Float32Array) {
    if (this.original) this.original.normals = normals;
    if (!this.mesh) return;
    this.mesh.geometry.setAttribute('normal', new THREE.BufferAttribute(normals, 3));
    this.invalidate();
  }

  /**
   * Apply print pose (defines the build frame: min Z = 0, XY centered) and
   * display pose (presentation only, re-rested on the floor).
   */
  setPose(pose: PoseSettings) {
    if (!this.mesh || !this.welded) return;
    const v = new THREE.Vector3();
    const p = this.welded;
    const R = eulerMatrix(pose.print);
    const min = new THREE.Vector3(Infinity, Infinity, Infinity), max = new THREE.Vector3(-Infinity, -Infinity, -Infinity);
    for (let i = 0; i < p.length; i += 3) {
      v.set(p[i], p[i + 1], p[i + 2]).applyMatrix4(R);
      min.min(v); max.max(v);
    }
    const printM = new THREE.Matrix4().makeTranslation(-(min.x + max.x) / 2, -(min.y + max.y) / 2, -min.z).multiply(R);
    this.mesh.matrix.copy(printM);
    this.layerUniforms.uPrint.value.copy(printM);
    this.inspector.setModelMatrix(printM);
    this.buildSize = [max.x - min.x, max.y - min.y, max.z - min.z];

    const D = eulerMatrix(pose.display).multiply(printM);
    min.set(Infinity, Infinity, Infinity); max.set(-Infinity, -Infinity, -Infinity);
    for (let i = 0; i < p.length; i += 3) {
      v.set(p[i], p[i + 1], p[i + 2]).applyMatrix4(D);
      min.min(v); max.max(v);
    }
    const cx = (min.x + max.x) / 2, cy = (min.y + max.y) / 2;
    const displayM = new THREE.Matrix4().makeTranslation(-cx, -cy, -min.z).multiply(eulerMatrix(pose.display));
    this.display.matrix.copy(displayM);
    this.display.matrixWorldNeedsUpdate = true;

    let radius = 0;
    for (let i = 0; i < p.length; i += 3) {
      v.set(p[i], p[i + 1], p[i + 2]).applyMatrix4(D);
      radius = Math.max(radius, Math.hypot(v.x - cx, v.y - cy));
    }
    this.envelope = { radius, height: max.z - min.z, size: [max.x - min.x, max.y - min.y, max.z - min.z] };
    this.layoutStage();
    this.invalidate();
  }

  /** Size backdrop, lights and shadow camera to the model envelope. */
  private layoutStage() {
    this.updateAoScale();
    const { radius, height } = this.envelope;
    const s = Math.max(radius, height, 1);
    this.backdrop.geometry.dispose();
    this.backdrop.geometry = sweepGeometry(s * 60, s * 30, s * 6, s * 40);
    this.backdrop.position.set(0, radius * 2.5 + s * 0.5, 0);

    const target = new THREE.Vector3(0, 0, height * 0.4);
    const place = (light: THREE.DirectionalLight, dir: Vec3) => {
      light.position.set(dir[0], dir[1], dir[2]).normalize().multiplyScalar(s * 8).add(target);
      light.target.position.copy(target);
    };
    place(this.key, [-0.9, -1.1, 1.5]);
    place(this.fill, [1.3, -0.7, 0.45]);
    place(this.rim, [0.4, 1.6, 1.1]);
    const cam = this.key.shadow.camera;
    const r = Math.hypot(radius, height) * 1.3 + 1;
    cam.left = -r; cam.right = r; cam.top = r; cam.bottom = -r;
    cam.near = 0.1; cam.far = s * 20;
    cam.updateProjectionMatrix();
    this.key.shadow.normalBias = s * 0.0015;
    this.camera.near = Math.max(0.01, s * 0.01);
    this.camera.far = s * 200;
    this.camera.updateProjectionMatrix();
    this.ortho.near = Math.max(0.01, s * 0.01);
    this.ortho.far = s * 200;
    this.ortho.updateProjectionMatrix();
  }

  // ------------------------------------------------------------- appearance

  private lastMaterial: MaterialSettings | null = null;
  private lastLayers: LayerSettings | null = null;
  private lastStudio: StudioSettings | null = null;

  setMaterial(m: MaterialSettings) {
    this.lastMaterial = m;
    for (const mat of [this.material, this.beadMaterial]) {
      mat.color.set(m.color);
      mat.roughness = m.roughness;
      mat.metalness = m.metalness;
      mat.clearcoat = m.clearcoat;
      mat.clearcoatRoughness = m.clearcoatRoughness;
      mat.sheen = m.sheen;
      mat.sheenColor.set(m.color).lerp(new THREE.Color(1, 1, 1), 0.5);
      mat.sheenRoughness = 0.6;
      mat.transmission = m.transmission;
      mat.thickness = m.thickness;
      mat.attenuationDistance = m.attenuationDistance;
      mat.attenuationColor.set(m.color);
      mat.ior = m.ior;
    }
    this.invalidate();
  }

  // ------------------------------------------------------------ G-code beads

  readonly beadUniforms: BeadUniforms = createBeadUniforms();
  readonly beadMaterial = new THREE.MeshPhysicalMaterial({ color: 0x3d7be0, roughness: 0.6, side: THREE.FrontSide });
  private beadDepth = new THREE.MeshDepthMaterial({ depthPacking: THREE.RGBADepthPacking });
  private beadsReady = false;
  private beadNormalDepthReady = false;
  modelKind: 'mesh' | 'beads' = 'mesh';
  beadCount = 0;

  private initBeadMaterials() {
    if (this.beadsReady) return;
    applyBeadShader(this.beadMaterial, this.beadUniforms, true);
    applyBeadShader(this.beadDepth, this.beadUniforms, false);
    this.beadsReady = true;
  }

  /**
   * Show a G-code toolpath as instanced beads. `points` (bead endpoints) feed
   * the bounds/pose logic shared with STL models.
   */
  setBeads(data: BeadData, layerCount: number, points: Float32Array) {
    this.initBeadMaterials();
    if (this.mesh) {
      this.display.remove(this.mesh);
      this.mesh.geometry.dispose();
    }
    this.original = null;
    const g = createBeadGeometry(data);
    let r = 0;
    for (let i = 0; i < points.length; i += 3) r = Math.max(r, Math.hypot(points[i], points[i + 1], points[i + 2]));
    g.boundingSphere = new THREE.Sphere(new THREE.Vector3(), r + 5);
    const mesh = new THREE.Mesh(g, this.beadMaterial);
    mesh.customDepthMaterial = this.beadDepth;
    mesh.renderOrder = 6;
    mesh.castShadow = true;
    mesh.receiveShadow = true;
    mesh.matrixAutoUpdate = false;
    mesh.frustumCulled = false;
    this.mesh = mesh;
    this.welded = points;
    this.modelKind = 'beads';
    this.beadCount = data.count;
    this.beadUniforms.uLayerCount.value = layerCount;
    this.beadUniforms.uProgress.value = 1e12;
    this.inspector.setGeometry(g, this.beadUniforms);
    this.display.add(mesh);
    this.invalidate();
  }

  setBeadDisplay(o: { colorMode: BeadColorMode; hiddenMask: number; featureColors: string[]; toolColors: string[] }) {
    const u = this.beadUniforms;
    u.uColorMode.value = { material: 0, feature: 1, tool: 2, layer: 3 }[o.colorMode];
    u.uHiddenMask.value = o.hiddenMask;
    o.featureColors.forEach((c, i) => i < 32 && u.uFeatureColors.value[i].set(c));
    o.toolColors.forEach((c, i) => i < 8 && u.uToolColors.value[i].set(c));
    this.invalidate();
  }

  private userInspection: InspectionSettings | null = null;
  private buildProgress: number | null = null;

  /**
   * Build-up progress in [0, 1] (null = complete). G-code plays back bead by
   * bead; meshes are cut by a rising capped Z plane.
   */
  setBuildProgress(p: number | null) {
    this.buildProgress = p;
    if (this.modelKind === 'beads') {
      this.beadUniforms.uProgress.value = p === null ? 1e12 : p * this.beadCount;
      this.invalidate('transform');
      return;
    } else if (this.userInspection) {
      const base = this.userInspection;
      const s: InspectionSettings = p === null ? base : {
        ...base,
        section: { ...base.section, enabled: true, axis: 'z', flip: false, offsetMm: Math.max(1e-4, p) * this.buildSize[2], capColor: this.lastMaterial?.color ?? base.section.capColor },
      };
      this.applyInspection(s);
    }
    this.invalidate();
  }

  private boundsTexture: THREE.DataTexture | null = null;

  /** Variable layer boundaries (build-frame mm), or null for uniform heights. */
  setLayerSchedule(boundaries: Float64Array | number[] | null) {
    this.boundaries = boundaries && boundaries.length >= 2 ? Array.from(boundaries) : null;
    const u = this.layerUniforms;
    this.boundsTexture?.dispose();
    this.boundsTexture = null;
    if (!boundaries || boundaries.length < 2) {
      u.uVarOn.value = 0;
      this.invalidate();
      return;
    }
    const n = boundaries.length;
    const w = Math.min(n, 4096), h = Math.ceil(n / w);
    const data = new Float32Array(w * h);
    for (let i = 0; i < n; i++) data[i] = boundaries[i];
    const tex = new THREE.DataTexture(data, w, h, THREE.RedFormat, THREE.FloatType);
    tex.needsUpdate = true;
    this.boundsTexture = tex;
    u.uBounds.value = tex;
    u.uBoundCount.value = n;
    u.uBoundW.value = w;
    u.uVarOn.value = 1;
    this.invalidate();
  }

  setLayers(l: LayerSettings) {
    this.lastLayers = l;
    const u = this.layerUniforms;
    u.uLayerOn.value = l.enabled ? 1 : 0;
    u.uLayerH.value = l.heightMm;
    u.uFirstH.value = l.firstMm;
    u.uLayerDepth.value = l.depthMm;
    u.uLayerRound.value = l.roundness;
    u.uLineW.value = l.lineWidthMm;
    u.uTopOn.value = l.topLines ? 1 : 0;
    this.invalidate();
  }

  setStudio(s: StudioSettings) {
    this.lastStudio = s;
    const bg = new THREE.Color(s.background);
    this.scene.background = bg;
    this.backdropMat.color.copy(bg);
    // The softbox environment carries most of the light; direct lights add
    // crisp shadowing and a little extra shape.
    this.scene.environmentIntensity = (s.keyIntensity / 2.4) * (0.55 + s.envIntensity * 0.8);
    this.key.intensity = s.keyIntensity * 0.45;
    this.fill.intensity = s.keyIntensity * 0.08;
    this.rim.intensity = s.keyIntensity * 0.25;
    this.key.castShadow = s.shadows;
    this.aoEnabled = s.ao;
    this.invalidate();
  }

  setInspection(s: InspectionSettings) {
    this.userInspection = s;
    if (this.buildProgress !== null && this.modelKind === 'mesh') this.setBuildProgress(this.buildProgress);
    else this.applyInspection(s);
  }

  /** Run `fn` with temporarily modified inspection settings, then restore. */
  withInspection<T>(modify: (s: InspectionSettings) => InspectionSettings, fn: () => T): T {
    const base = this.userInspection ?? this.inspector.settings;
    this.applyInspection(modify(base));
    try {
      return fn();
    } finally {
      this.applyInspection(base);
    }
  }

  private applyInspection(s: InspectionSettings) {
    this.inspector.apply(s, this.material.color);
    // Beads share the section clip plane (caps and see-through are mesh-only).
    this.beadMaterial.clippingPlanes = s.section.enabled ? [this.inspector.worldPlane] : [];
    this.beadMaterial.clipShadows = s.section.enabled;
    this.beadMaterial.needsUpdate = true;
    this.invalidate();
  }

  /** Build-up as a G-code layer threshold for the path tracer (null = complete). */
  beadBuildLayer(): number | null {
    return this.modelKind === 'beads' && this.buildProgress !== null ? this.buildProgress * this.beadUniforms.uLayerCount.value : null;
  }

  /** Render the scene, including section caps and see-through compositing. */
  // Ambient occlusion (GTAO) post-processing for the raster engine.
  private composer: EffectComposer | null = null;
  private renderPass: RenderPass | null = null;
  private gtao: GTAOPass | null = null;
  aoEnabled = true;
  private drawTransparent = false;

  private aoActive(): boolean {
    // AO is skipped for translucent materials: occlusion of the front surface is
    // physically meaningless there and its noise crawls while the camera moves.
    return this.aoEnabled && !!this.mesh && this.modelKind === 'mesh' && this.material.transmission === 0 &&
      !this.inspector.seeThroughActive && !this.inspector.settings.section.enabled && !this.drawTransparent;
  }

  private ensureComposer() {
    if (this.composer) return;
    const target = new THREE.WebGLRenderTarget(1, 1, { type: THREE.HalfFloatType, samples: 4 });
    this.composer = new EffectComposer(this.renderer, target);
    this.renderPass = new RenderPass(this.scene, this.camera);
    this.gtao = new GTAOPass(this.scene, this.camera, 1, 1);
    this.gtao.blendIntensity = 1;
    this.composer.addPass(this.renderPass);
    this.composer.addPass(this.gtao);
    this.composer.addPass(new OutputPass());
    this.updateAoScale();
  }

  private updateAoScale() {
    if (!this.gtao) return;
    const s = Math.max(this.envelope.radius, this.envelope.height, 1);
    // Conservative, millimetre-capped AO: large radii reach across deep openings
    // and produce blocky, flickering dark patches after denoising.
    this.gtao.updateGtaoMaterial({ radius: Math.min(s * 0.12, 8), thickness: Math.min(s * 0.02, 1.5), distanceExponent: 1, scale: 1 });
    this.gtao.updatePdMaterial({ lumaPhi: 10, depthPhi: 2, normalPhi: 3, radius: 6, rings: 2, samples: 16 });
  }

  private draw(camera: THREE.Camera) {
    const scale = Math.max(this.envelope.radius, this.envelope.height, 1);
    this.inspector.update(scale * 6);
    if (this.aoActive()) {
      this.ensureComposer();
      const size = this.renderer.getDrawingBufferSize(new THREE.Vector2());
      const c = this.composer!;
      if (c.renderTarget1.width !== size.x || c.renderTarget1.height !== size.y) {
        c.setPixelRatio(1);
        c.setSize(size.x, size.y);
      }
      this.renderPass!.camera = camera;
      this.gtao!.camera = camera;
      c.render();
      return;
    }
    if (this.mesh && this.inspector.seeThroughActive) {
      this.mesh.visible = false;
      this.renderer.render(this.scene, camera);
      this.mesh.visible = true;
      this.inspector.renderSeeThrough(this.renderer, camera, this.mesh, scale);
    } else {
      this.renderer.render(this.scene, camera);
    }
  }

  setTurntableAngle(deg: number) {
    if (this.turntable.rotation.z === deg * DEG) return;
    this.turntable.rotation.z = deg * DEG;
    this.invalidate('transform');
  }

  // ---------------------------------------------------------------- cameras

  /** Fit the perspective camera to the full rotational envelope. */
  // ------------------------------------------------------- export framing

  /** Aspect (w/h) of the active export, shown as a frame in the viewport; null = no frame. */
  exportAspect: number | null = 1;
  /** Gap (CSS px) kept between the frame and the viewport edge. */
  static readonly FRAME_MARGIN = 24;

  /** The export frame inside the canvas (CSS px), centred; null without an export aspect. */
  frameRect(): { x: number; y: number; w: number; h: number } | null {
    const W = this.canvas.clientWidth, H = this.canvas.clientHeight;
    if (!this.exportAspect || W === 0 || H === 0) return null;
    const m = Studio.FRAME_MARGIN;
    const aw = Math.max(1, W - 2 * m), ah = Math.max(1, H - 2 * m);
    const h = Math.min(ah, aw / this.exportAspect);
    const w = h * this.exportAspect;
    return { x: (W - w) / 2, y: (H - h) / 2, w, h };
  }

  /** Vertical field of view (deg) and aspect seen through the export frame. */
  private frameView(): { fov: number; aspect: number } {
    const r = this.frameRect();
    const H = this.canvas.clientHeight;
    if (!r || H === 0) return { fov: this.camera.fov, aspect: this.camera.aspect };
    const t = Math.tan((this.camera.fov * Math.PI) / 360) * (r.h / H);
    return { fov: (Math.atan(t) * 360) / Math.PI, aspect: r.w / r.h };
  }

  /**
   * Camera for exports: the viewport camera cropped to the export frame (same
   * pose, narrower field of view), so renders contain exactly what the frame shows.
   */
  exportCamera(): THREE.PerspectiveCamera {
    const v = this.frameView();
    const cam = this.camera.clone();
    cam.fov = v.fov;
    cam.aspect = v.aspect;
    cam.updateProjectionMatrix();
    cam.updateMatrixWorld(true);
    return cam;
  }

  /** Fit the full rotational envelope into the export frame (or the viewport). */
  fitHero(azimuth = 0, elevation = 20, margin = 0.1) {
    const { radius, height } = this.envelope;
    const v = this.frameView();
    const fit = fitPerspective(envelopePoints(radius, 0, height), directionFromAngles(azimuth, elevation), v.fov, v.aspect, margin);
    this.applyPerspective(fit.position, fit.target);
  }

  applyPerspective(position: Vec3, target: Vec3) {
    this.useOrtho = false;
    this.controls.object = this.camera;
    this.camera.position.set(...position);
    this.camera.up.set(0, 0, 1);
    this.controls.target.set(...target);
    this.camera.lookAt(this.controls.target);
    this.controls.update();
    this.invalidate();
  }

  setView(view: ViewName) {
    const dirs: Record<ViewName, [number, number]> = { front: [0, 0], right: [90, 0], top: [0, 89.9], iso: [45, 35.264], hero: [0, 20] };
    const [az, el] = dirs[view];
    this.fitHero(az, el, 0.12);
  }

  // ----------------------------------------------------------------- export

  /** Build-frame vertices of the current model (print pose applied), subsampled. */
  private buildFramePoints(): Vec3[] {
    if (!this.mesh || !this.welded) return [];
    const m = this.mesh.matrix;
    const v = new THREE.Vector3();
    const pts: Vec3[] = [];
    const p = this.welded;
    const stride = Math.max(1, Math.floor(p.length / 3 / 400_000)) * 3;
    for (let i = 0; i < p.length; i += stride) {
      v.set(p[i], p[i + 1], p[i + 2]).applyMatrix4(m);
      pts.push([v.x, v.y, v.z]);
    }
    return pts;
  }

  /** Place key/fill/rim relative to a camera basis so every view is well lit. */
  private lightForView(f: THREE.Vector3, r: THREE.Vector3, u: THREE.Vector3, target: THREE.Vector3, dist: number) {
    const place = (light: THREE.DirectionalLight, a: number, b: number, c: number) => {
      light.position.copy(f).multiplyScalar(a).addScaledVector(r, b).addScaledVector(u, c).normalize().multiplyScalar(dist).add(target);
      light.target.position.copy(target);
      light.target.updateMatrixWorld();
    };
    place(this.key, -1.0, -0.7, 0.9);
    place(this.fill, -0.8, 0.9, 0.1);
    place(this.rim, 0.9, 0.2, 0.6);
  }

  /**
   * Render the four orthographic panels in the build frame at one shared
   * scale. Returns shaded RGBA panels and normal/depth buffers for edges.
   */
  renderFourView(o: FourViewRenderOptions): FourViewPanels {
    if (!this.mesh) throw new Error('No model loaded');
    const saved = {
      display: this.display.matrix.clone(),
      angle: this.turntable.rotation.z,
      material: this.lastMaterial,
      layers: this.lastLayers,
      studio: this.lastStudio,
    };
    const pts = this.buildFramePoints();
    const min: Vec3 = [Infinity, Infinity, Infinity], max: Vec3 = [-Infinity, -Infinity, -Infinity];
    for (const p of pts) for (let k = 0; k < 3; k++) { min[k] = Math.min(min[k], p[k]); max[k] = Math.max(max[k], p[k]); }
    const center: Vec3 = [(min[0] + max[0]) / 2, (min[1] + max[1]) / 2, (min[2] + max[2]) / 2];
    const radius = Math.hypot(max[0] - min[0], max[1] - min[1], max[2] - min[2]) / 2;

    let mmPerPx = 0;
    for (const view of PANEL_ORDER) {
      const { dir, up } = PANEL_DIRS[view];
      const e = orthoExtents(pts, center, dir, up);
      mmPerPx = Math.max(mmPerPx, (2 * e.halfW) / (o.imageW * (1 - 2 * o.margin)), (2 * e.halfH) / (o.imageH * (1 - 2 * o.margin)));
    }

    // Presentation overrides: build frame, no floor/shadows, neutral finish.
    this.display.matrix.identity();
    this.display.matrixWorldNeedsUpdate = true;
    this.turntable.rotation.z = 0;
    this.setMaterial(o.material);
    this.setLayers(o.layers);
    const bg = this.scene.background;
    const backdropVisible = this.backdrop.visible;
    const shadow = this.key.castShadow;
    this.scene.background = new THREE.Color(o.background);
    this.backdrop.visible = false;
    this.key.castShadow = false;

    const shaded: Uint8ClampedArray[] = [];
    const normalDepth: Float32Array[] = [];
    const cam = new THREE.OrthographicCamera(-1, 1, 1, -1, 0.01, radius * 10 + 10);
    const target = new THREE.Vector3(...center);
    try {
      for (const view of PANEL_ORDER) {
        const { dir, up } = PANEL_DIRS[view];
        const f = new THREE.Vector3(...dir).normalize();
        let r = new THREE.Vector3().crossVectors(f, new THREE.Vector3(...up));
        if (r.lengthSq() < 1e-9) r = new THREE.Vector3(1, 0, 0);
        r.normalize();
        const u = new THREE.Vector3().crossVectors(r, f).normalize();
        const dist = radius * 4 + 10;
        cam.position.copy(target).addScaledVector(f, -dist);
        cam.up.copy(u);
        cam.lookAt(target);
        const halfW = (o.imageW * mmPerPx) / 2, halfH = (o.imageH * mmPerPx) / 2;
        cam.left = -halfW; cam.right = halfW; cam.top = halfH; cam.bottom = -halfH;
        cam.near = Math.max(0.01, dist - radius * 2);
        cam.far = dist + radius * 2;
        cam.updateProjectionMatrix();
        cam.updateMatrixWorld();
        this.lightForView(f, r, u, target, radius * 8 + 10);
        shaded.push(this.renderPixels(o.imageW, o.imageH, o.supersample, cam));
        normalDepth.push(this.renderNormalDepth(o.imageW, o.imageH, cam));
      }
    } finally {
      this.display.matrix.copy(saved.display);
      this.display.matrixWorldNeedsUpdate = true;
      this.turntable.rotation.z = saved.angle;
      this.scene.background = bg;
      this.backdrop.visible = backdropVisible;
      this.key.castShadow = shadow;
      if (saved.material) this.setMaterial(saved.material);
      if (saved.layers) this.setLayers(saved.layers);
      if (saved.studio) this.setStudio(saved.studio);
      this.layoutStage();
      this.invalidate();
    }
    return { shaded, normalDepth, mmPerPx, sizeMm: [max[0] - min[0], max[1] - min[1], max[2] - min[2]] };
  }

  /** View-space normal (xyz) and depth in mm (w; -1 = background), top-down rows. */
  renderNormalDepth(width: number, height: number, camera: THREE.Camera): Float32Array {
    const r = this.renderer;
    const rt = new THREE.WebGLRenderTarget(width, height, { type: THREE.FloatType, format: THREE.RGBAFormat, depthBuffer: true });
    const prevOverride = this.scene.overrideMaterial;
    const prevBg = this.scene.background;
    const backdrop = this.backdrop.visible;
    const prevClear = new THREE.Color();
    r.getClearColor(prevClear);
    const prevAlpha = r.getClearAlpha();
    // Beads need their instancing shader, so they swap material instead of using the override.
    const beads = this.modelKind === 'beads' && this.mesh;
    const prevMeshMaterial = this.mesh?.material;
    if (beads) {
      if (!this.beadNormalDepthReady) {
        applyBeadShader(beadNormalDepthMaterial, this.beadUniforms, false);
        this.beadNormalDepthReady = true;
      }
      beadNormalDepthMaterial.clippingPlanes = this.inspector.settings.section.enabled ? [this.inspector.worldPlane] : [];
      this.mesh!.material = beadNormalDepthMaterial;
    } else {
      this.scene.overrideMaterial = normalDepthMaterial;
    }
    normalDepthMaterial.clippingPlanes = this.inspector.settings.section.enabled ? [this.inspector.worldPlane] : [];
    this.inspector.update(Math.max(this.envelope.radius, this.envelope.height, 1) * 6);
    const capVisible = this.inspector.capMesh.visible;
    this.inspector.capMesh.visible = false;
    this.scene.background = null;
    this.backdrop.visible = false;
    r.setRenderTarget(rt);
    r.setClearColor(0x000000, -1);
    r.clear();
    r.render(this.scene, camera);
    const raw = new Float32Array(width * height * 4);
    r.readRenderTargetPixels(rt, 0, 0, width, height, raw);
    r.setRenderTarget(null);
    r.setClearColor(prevClear, prevAlpha);
    this.scene.overrideMaterial = prevOverride;
    if (beads && prevMeshMaterial) this.mesh!.material = prevMeshMaterial;
    this.inspector.capMesh.visible = capVisible;
    this.scene.background = prevBg;
    this.backdrop.visible = backdrop;
    rt.dispose();
    // Flip rows to top-down.
    const out = new Float32Array(raw.length);
    const row = width * 4;
    for (let y = 0; y < height; y++) out.set(raw.subarray((height - 1 - y) * row, (height - y) * row), y * row);
    return out;
  }


  /**
   * Render the current scene at an explicit size and return top-down RGBA
   * pixels. `supersample` renders larger and box-filters down.
   */
  renderPixels(width: number, height: number, supersample = 1, camera: THREE.Camera = this.camera, transparent = false): Uint8ClampedArray {
    const r = this.renderer;
    const prevBackground = this.scene.background;
    const prevBackdrop = this.backdrop.visible;
    if (transparent) {
      this.scene.background = null;
      this.backdrop.visible = false;
      r.setClearColor(0x000000, 0);
    }
    this.drawTransparent = transparent;
    const prevRatio = r.getPixelRatio();
    const prevSize = r.getSize(new THREE.Vector2());
    const ss = supersample;
    const W = width * ss, H = height * ss;
    r.setPixelRatio(1);
    let restoreAspect: number | null = null;
    if (camera instanceof THREE.PerspectiveCamera) {
      restoreAspect = camera.aspect;
      camera.aspect = width / height;
      camera.updateProjectionMatrix();
    }
    // Layer fade thresholds are in output pixels, not supersampled ones.
    const u = this.layerUniforms;
    const fade = [u.uFadeLo.value, u.uFadeHi.value];
    u.uFadeLo.value = fade[0] * ss;
    u.uFadeHi.value = fade[1] * ss;

    // Browsers silently cap the canvas drawing buffer, so large renders are
    // tiled with a camera view offset and stitched.
    const gl = r.getContext();
    const tile = Math.max(1, Math.floor(TILE_PX / ss)); // output pixels per tile edge
    const out = new Uint8ClampedArray(width * height * 4);
    const persp = camera as THREE.PerspectiveCamera | THREE.OrthographicCamera;
    const tiled = width > tile || height > tile;
    try {
      for (let ty = 0; ty < height; ty += tile) {
        for (let tx = 0; tx < width; tx += tile) {
          const tw = Math.min(tile, width - tx), th = Math.min(tile, height - ty);
          r.setSize(tw * ss, th * ss, false);
          if (gl.drawingBufferWidth < tw * ss || gl.drawingBufferHeight < th * ss) {
            throw new Error(`GPU drawing buffer limited to ${gl.drawingBufferWidth}×${gl.drawingBufferHeight}`);
          }
          if (tiled) persp.setViewOffset(W, H, tx * ss, ty * ss, tw * ss, th * ss);
          this.draw(camera);
          const raw = new Uint8Array(tw * ss * th * ss * 4);
          gl.readPixels(0, 0, tw * ss, th * ss, gl.RGBA, gl.UNSIGNED_BYTE, raw);
          const part = downsampleFlip(raw, tw * ss, th * ss, ss);
          for (let y = 0; y < th; y++) out.set(part.subarray(y * tw * 4, (y + 1) * tw * 4), ((ty + y) * width + tx) * 4);
        }
      }
    } finally {
      if (tiled) persp.clearViewOffset();
      u.uFadeLo.value = fade[0];
      u.uFadeHi.value = fade[1];
      if (restoreAspect !== null && camera instanceof THREE.PerspectiveCamera) {
        camera.aspect = restoreAspect;
        camera.updateProjectionMatrix();
      }
      r.setPixelRatio(prevRatio);
      r.setSize(prevSize.x, prevSize.y, false);
      if (transparent) {
        this.scene.background = prevBackground;
        this.backdrop.visible = prevBackdrop;
        r.setClearColor(0x000000, 1);
      }
      this.drawTransparent = false;
      this.invalidate();
    }
    if (transparent) unpremultiply(out);
    return out;
  }
}

/** Convert premultiplied RGBA (WebGL default framebuffer) to straight alpha. */
export function unpremultiply(px: Uint8ClampedArray) {
  for (let i = 0; i < px.length; i += 4) {
    const a = px[i + 3];
    if (a > 0 && a < 255) {
      px[i] = (px[i] * 255) / a;
      px[i + 1] = (px[i + 1] * 255) / a;
      px[i + 2] = (px[i + 2] * 255) / a;
    }
  }
}

/** Flip bottom-up GL pixels and box-filter by an integer factor. */
export function downsampleFlip(raw: Uint8Array, W: number, H: number, k: number): Uint8ClampedArray {
  const w = Math.floor(W / k), h = Math.floor(H / k);
  const out = new Uint8ClampedArray(w * h * 4);
  const n = k * k;
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      let r = 0, g = 0, b = 0, a = 0;
      for (let dy = 0; dy < k; dy++) {
        const sy = H - 1 - (y * k + dy);
        let o = (sy * W + x * k) * 4;
        for (let dx = 0; dx < k; dx++, o += 4) {
          r += raw[o]; g += raw[o + 1]; b += raw[o + 2]; a += raw[o + 3];
        }
      }
      const d = (y * w + x) * 4;
      out[d] = r / n; out[d + 1] = g / n; out[d + 2] = b / n; out[d + 3] = a / n;
    }
  }
  return out;
}
