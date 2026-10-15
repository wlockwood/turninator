// Inspection tools for the Studio renderer:
//  - Section plane in the build frame with stencil-capped cross-sections.
//  - See-through (X-ray) using weighted blended order-independent
//    transparency (McGuire & Bavoil 2013) in two single-target passes, so no
//    per-attachment blend state is needed. Optional feature-edge overlay.

import * as THREE from 'three';
import { applyBeadShader, type BeadUniforms } from './beads';

export type Axis = 'x' | 'y' | 'z';

export interface SectionSettings {
  enabled: boolean;
  axis: Axis;
  /** Cut position along the axis in build-frame mm. */
  offsetMm: number;
  /** Keep the positive side instead of the negative side. */
  flip: boolean;
  capColor: string;
}

export interface SeeThroughSettings {
  enabled: boolean;
  opacity: number;
  edges: boolean;
}

export interface InspectionSettings {
  section: SectionSettings;
  seeThrough: SeeThroughSettings;
}

export const DEFAULT_INSPECTION: InspectionSettings = {
  section: { enabled: false, axis: 'y', offsetMm: 0, flip: false, capColor: '#e05a47' },
  seeThrough: { enabled: false, opacity: 0.25, edges: true },
};

/** +1 keeps coordinates above the cut, -1 keeps those below. */
const KEEP_SIGN: Record<Axis, number> = { x: -1, y: 1, z: -1 };

const AXES: Record<Axis, THREE.Vector3> = { x: new THREE.Vector3(1, 0, 0), y: new THREE.Vector3(0, 1, 0), z: new THREE.Vector3(0, 0, 1) };

const XRAY_VERTEX = /* glsl */ `
#include <common>
#include <clipping_planes_pars_vertex>
varying vec3 vNormalV;
varying vec3 vViewDir;
varying float vViewZ;
void main() {
  // Standard chunks so instanced bead shaders can inject their transform.
  #include <beginnormal_vertex>
  #include <begin_vertex>
  #include <project_vertex>
  #include <clipping_planes_vertex>
  vNormalV = normalize(normalMatrix * objectNormal);
  vViewDir = normalize(-mvPosition.xyz);
  vViewZ = -mvPosition.z;
}`;

const XRAY_FRAGMENT_COMMON = /* glsl */ `
#include <common>
#include <clipping_planes_pars_fragment>
uniform vec3 uColor;
uniform vec3 uBackColor;
uniform float uOpacity;
uniform float uRim;
uniform float uDepthScale;
varying vec3 vNormalV;
varying vec3 vViewDir;
varying float vViewZ;
vec4 xrayShade() {
  vec3 n = normalize(vNormalV) * (gl_FrontFacing ? 1.0 : -1.0);
  float facing = abs(dot(n, normalize(vViewDir)));
  float rim = pow(1.0 - facing, 2.5);
  vec3 light = normalize(vec3(-0.4, 0.6, 0.7));
  float diff = 0.45 + 0.55 * max(dot(n, light), 0.0);
  vec3 base = gl_FrontFacing ? uColor : uBackColor;
  vec3 color = base * diff + rim * 0.35;
  float a = clamp(uOpacity + (1.0 - uOpacity) * rim * uRim, 0.0, 0.98);
  return vec4(color, a);
}
float oitWeight(float z, float a) {
  float d = z * uDepthScale;
  return a * max(1e-2, min(3e3, 10.0 / (1e-5 + pow(d / 5.0, 2.0) + pow(d / 200.0, 6.0))));
}`;

const ACCUM_FRAGMENT = /* glsl */ `
${XRAY_FRAGMENT_COMMON}
void main() {
  #include <clipping_planes_fragment>
  vec4 c = xrayShade();
  float w = oitWeight(vViewZ, c.a);
  gl_FragColor = vec4(c.rgb * c.a, c.a) * w;
}`;

const REVEAL_FRAGMENT = /* glsl */ `
${XRAY_FRAGMENT_COMMON}
void main() {
  #include <clipping_planes_fragment>
  vec4 c = xrayShade();
  gl_FragColor = vec4(c.a);
}`;

const COMPOSITE_FRAGMENT = /* glsl */ `
uniform sampler2D tAccum;
uniform sampler2D tReveal;
varying vec2 vUv;
void main() {
  vec4 accum = texture2D(tAccum, vUv);
  float reveal = texture2D(tReveal, vUv).r;
  if (reveal >= 0.9999) discard;
  vec3 color = accum.rgb / clamp(accum.a, 1e-4, 5e4);
  gl_FragColor = vec4(color, 1.0 - reveal);
  #include <tonemapping_fragment>
  #include <colorspace_fragment>
}`;

export class Inspector {
  settings: InspectionSettings = structuredClone(DEFAULT_INSPECTION);
  /** Build-frame plane (in the display group's local space). */
  private buildPlane = new THREE.Plane();
  /** World-space plane shared by all clipped materials (updated per render). */
  readonly worldPlane = new THREE.Plane();
  private stencilBack: THREE.Mesh;
  private stencilFront: THREE.Mesh;
  private cap: THREE.Mesh;
  private capMat: THREE.MeshStandardMaterial;
  private edgesGeometry: THREE.EdgesGeometry | null = null;
  private edgeScene = new THREE.Scene();
  private edgeProxy: THREE.LineSegments;
  private edgesMat = new THREE.LineBasicMaterial({ color: 0x1d2633, transparent: true, opacity: 0.55, depthTest: false });
  private xrayScene = new THREE.Scene();
  private xrayMesh: THREE.Mesh;
  private accumMat: THREE.ShaderMaterial;
  private revealMat: THREE.ShaderMaterial;
  private compositeScene = new THREE.Scene();
  private compositeCam = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 1);
  private compositeMat: THREE.ShaderMaterial;
  private accumRT: THREE.WebGLRenderTarget | null = null;
  private revealRT: THREE.WebGLRenderTarget | null = null;
  private geometry: THREE.BufferGeometry | null = null;

  constructor(private display: THREE.Group, private modelMaterial: THREE.Material) {
    const base = new THREE.MeshBasicMaterial({ depthWrite: false, depthTest: false, colorWrite: false, stencilWrite: true, stencilFunc: THREE.AlwaysStencilFunc });
    const back = base.clone();
    back.side = THREE.BackSide;
    back.clippingPlanes = [this.worldPlane];
    back.stencilFail = back.stencilZFail = back.stencilZPass = THREE.IncrementWrapStencilOp;
    const front = base.clone();
    front.side = THREE.FrontSide;
    front.clippingPlanes = [this.worldPlane];
    front.stencilFail = front.stencilZFail = front.stencilZPass = THREE.DecrementWrapStencilOp;
    this.stencilBack = new THREE.Mesh(new THREE.BufferGeometry(), back);
    this.stencilFront = new THREE.Mesh(new THREE.BufferGeometry(), front);
    for (const m of [this.stencilBack, this.stencilFront]) {
      m.matrixAutoUpdate = false;
      m.renderOrder = 1;
      m.visible = false;
      display.add(m);
    }

    this.capMat = new THREE.MeshStandardMaterial({
      color: 0xe05a47, roughness: 0.8, metalness: 0, side: THREE.DoubleSide,
      stencilWrite: true, stencilRef: 0, stencilFunc: THREE.NotEqualStencilFunc,
      stencilFail: THREE.ReplaceStencilOp, stencilZFail: THREE.ReplaceStencilOp, stencilZPass: THREE.ReplaceStencilOp,
    });
    this.cap = new THREE.Mesh(new THREE.PlaneGeometry(1, 1), this.capMat);
    this.cap.renderOrder = 1.1;
    this.cap.visible = false;
    this.cap.onAfterRender = (renderer) => renderer.clearStencil();
    // The cap lives in world space so it can follow the plane through turntable rotation.

    const uniforms = {
      uColor: { value: new THREE.Color(0x7fb2ff) },
      uBackColor: { value: new THREE.Color(0xff9a6b) },
      uOpacity: { value: 0.25 },
      uRim: { value: 0.85 },
      uDepthScale: { value: 1 },
    };
    this.accumMat = new THREE.ShaderMaterial({
      uniforms, vertexShader: XRAY_VERTEX, fragmentShader: ACCUM_FRAGMENT, side: THREE.DoubleSide,
      transparent: true, depthWrite: false, depthTest: false, clipping: true,
      blending: THREE.CustomBlending, blendSrc: THREE.OneFactor, blendDst: THREE.OneFactor,
      blendSrcAlpha: THREE.OneFactor, blendDstAlpha: THREE.OneFactor,
    });
    this.revealMat = new THREE.ShaderMaterial({
      uniforms, vertexShader: XRAY_VERTEX, fragmentShader: REVEAL_FRAGMENT, side: THREE.DoubleSide,
      transparent: true, depthWrite: false, depthTest: false, clipping: true,
      blending: THREE.CustomBlending, blendSrc: THREE.ZeroFactor, blendDst: THREE.OneMinusSrcAlphaFactor,
      blendSrcAlpha: THREE.ZeroFactor, blendDstAlpha: THREE.OneMinusSrcAlphaFactor,
    });
    this.accumMat.clippingPlanes = [this.worldPlane];
    this.revealMat.clippingPlanes = [this.worldPlane];
    this.xrayMesh = new THREE.Mesh(new THREE.BufferGeometry(), this.accumMat);
    this.xrayMesh.matrixAutoUpdate = false;
    this.xrayMesh.frustumCulled = false;
    this.xrayScene.add(this.xrayMesh);

    this.compositeMat = new THREE.ShaderMaterial({
      uniforms: { tAccum: { value: null }, tReveal: { value: null } },
      vertexShader: 'varying vec2 vUv; void main() { vUv = uv; gl_Position = vec4(position.xy, 0.0, 1.0); }',
      fragmentShader: COMPOSITE_FRAGMENT,
      transparent: true, depthTest: false, depthWrite: false,
      blending: THREE.CustomBlending, blendSrc: THREE.SrcAlphaFactor, blendDst: THREE.OneMinusSrcAlphaFactor,
    });
    this.edgeProxy = new THREE.LineSegments(new THREE.BufferGeometry(), this.edgesMat);
    this.edgeProxy.matrixAutoUpdate = false;
    this.edgeProxy.frustumCulled = false;
    this.edgeScene.add(this.edgeProxy);
    const quad = new THREE.Mesh(new THREE.PlaneGeometry(2, 2), this.compositeMat);
    quad.frustumCulled = false;
    this.compositeScene.add(quad);
  }

  get capMesh() {
    return this.cap;
  }

  private plain: { back: THREE.Material; front: THREE.Material; accum: THREE.ShaderMaterial; reveal: THREE.ShaderMaterial } | null = null;
  private beadVariants: { back: THREE.Material; front: THREE.Material; accum: THREE.ShaderMaterial; reveal: THREE.ShaderMaterial } | null = null;
  private beads = false;

  /**
   * Use a model geometry for caps and see-through. Instanced G-code beads pass
   * their uniforms so the stencil and X-ray passes run the bead transform.
   */
  setGeometry(geometry: THREE.BufferGeometry, beadUniforms?: BeadUniforms) {
    this.geometry = geometry;
    this.stencilBack.geometry = geometry;
    this.stencilFront.geometry = geometry;
    this.xrayMesh.geometry = geometry;
    this.edgesGeometry?.dispose();
    this.edgesGeometry = null;
    this.plain ??= { back: this.stencilBack.material as THREE.Material, front: this.stencilFront.material as THREE.Material, accum: this.accumMat, reveal: this.revealMat };
    this.beads = !!beadUniforms;
    if (beadUniforms && !this.beadVariants) {
      const make = <T extends THREE.Material>(m: T): T => {
        const c = m.clone() as T;
        c.clippingPlanes = m.clippingPlanes;
        applyBeadShader(c, beadUniforms, false);
        return c;
      };
      const accum = make(this.plain.accum);
      const reveal = make(this.plain.reveal);
      // ShaderMaterial.clone copies uniforms; share the live ones instead.
      accum.uniforms = this.plain.accum.uniforms;
      reveal.uniforms = this.plain.reveal.uniforms;
      this.beadVariants = { back: make(this.plain.back), front: make(this.plain.front), accum, reveal };
    }
    const set = this.beads ? this.beadVariants! : this.plain;
    this.stencilBack.material = set.back;
    this.stencilFront.material = set.front;
    this.accumMat = set.accum;
    this.revealMat = set.reveal;
    this.xrayMesh.material = set.accum;
    for (const m of [this.stencilBack, this.stencilFront]) m.frustumCulled = !this.beads;
  }

  setModelMatrix(m: THREE.Matrix4) {
    this.stencilBack.matrix.copy(m);
    this.stencilFront.matrix.copy(m);
  }

  apply(s: InspectionSettings, modelColor: THREE.Color) {
    this.settings = s;
    const on = s.section.enabled;
    this.modelMaterial.clippingPlanes = on ? [this.worldPlane] : [];
    this.modelMaterial.clipShadows = on;
    this.modelMaterial.needsUpdate = true;
    this.accumMat.clippingPlanes = on ? [this.worldPlane] : [];
    this.revealMat.clippingPlanes = on ? [this.worldPlane] : [];
    const capVisible = on && !s.seeThrough.enabled;
    this.stencilBack.visible = this.stencilFront.visible = this.cap.visible = capVisible;
    this.capMat.color.set(s.section.capColor);
    // Keep the side where n·x + c >= 0. Defaults keep the half whose cut face
    // looks toward the default cameras (front -Y, right +X, top +Z).
    const sign = KEEP_SIGN[s.section.axis] * (s.section.flip ? -1 : 1);
    const n = AXES[s.section.axis].clone().multiplyScalar(sign);
    this.buildPlane.set(n, -sign * s.section.offsetMm);
    const u = this.accumMat.uniforms;
    // Toolpaths stack thousands of surfaces; per-surface opacity must be far lower.
    u.uOpacity.value = this.beads ? s.seeThrough.opacity * 0.08 : s.seeThrough.opacity;
    u.uRim.value = this.beads ? 0.05 : 0.85;
    u.uColor.value.copy(modelColor).lerp(new THREE.Color(1, 1, 1), 0.15);
    if (s.seeThrough.enabled && s.seeThrough.edges && !this.edgesGeometry && this.geometry && !this.beads) {
      this.edgesGeometry = new THREE.EdgesGeometry(this.geometry, 30);
      this.edgeProxy.geometry = this.edgesGeometry;
    }
  }

  /** Update world-space clip plane and cap placement; call before each render. */
  update(capSize: number) {
    this.display.updateMatrixWorld(true);
    this.worldPlane.copy(this.buildPlane).applyMatrix4(this.display.matrixWorld);
    if (this.cap.visible) {
      this.worldPlane.coplanarPoint(this.cap.position);
      const n = this.worldPlane.normal;
      this.cap.lookAt(this.cap.position.x - n.x, this.cap.position.y - n.y, this.cap.position.z - n.z);
      this.cap.scale.setScalar(capSize);
      this.cap.updateMatrixWorld(true);
    }
  }

  get seeThroughActive() {
    return this.settings.seeThrough.enabled;
  }

  /**
   * After the opaque scene (without the model) was rendered to the current
   * target, accumulate the model with WBOIT and composite on top.
   */
  renderSeeThrough(renderer: THREE.WebGLRenderer, camera: THREE.Camera, model: THREE.Mesh, sceneScale: number) {
    const size = renderer.getDrawingBufferSize(new THREE.Vector2());
    if (!this.accumRT || this.accumRT.width !== size.x || this.accumRT.height !== size.y) {
      this.accumRT?.dispose();
      this.revealRT?.dispose();
      this.accumRT = new THREE.WebGLRenderTarget(size.x, size.y, { type: THREE.HalfFloatType, depthBuffer: false });
      this.revealRT = new THREE.WebGLRenderTarget(size.x, size.y, { type: THREE.HalfFloatType, depthBuffer: false });
    }
    model.updateMatrixWorld();
    this.xrayMesh.matrix.copy(model.matrixWorld);
    this.xrayMesh.matrixWorld.copy(model.matrixWorld);
    this.accumMat.uniforms.uDepthScale.value = 100 / Math.max(sceneScale, 1e-3);

    const prevTarget = renderer.getRenderTarget();
    const prevAuto = renderer.autoClear;
    const prevClear = new THREE.Color();
    renderer.getClearColor(prevClear);
    const prevAlpha = renderer.getClearAlpha();
    const prevTone = renderer.toneMapping;
    renderer.autoClear = false;
    renderer.toneMapping = THREE.NoToneMapping; // accumulate linear values

    renderer.setRenderTarget(this.accumRT);
    renderer.setClearColor(0x000000, 0);
    renderer.clear(true, false, false);
    this.xrayMesh.material = this.accumMat;
    renderer.render(this.xrayScene, camera);

    renderer.setRenderTarget(this.revealRT);
    renderer.setClearColor(0xffffff, 1);
    renderer.clear(true, false, false);
    this.xrayMesh.material = this.revealMat;
    renderer.render(this.xrayScene, camera);

    renderer.toneMapping = prevTone;
    renderer.setRenderTarget(prevTarget);
    this.compositeMat.uniforms.tAccum.value = this.accumRT!.texture;
    this.compositeMat.uniforms.tReveal.value = this.revealRT!.texture;
    renderer.render(this.compositeScene, this.compositeCam);
    if (this.settings.seeThrough.edges && this.edgesGeometry) {
      // Feature-edge overlay drawn after compositing so lines stay crisp.
      this.edgeProxy.matrix.copy(model.matrixWorld);
      this.edgeProxy.matrixWorld.copy(model.matrixWorld);
      this.edgesMat.clippingPlanes = this.settings.section.enabled ? [this.worldPlane] : [];
      renderer.render(this.edgeScene, camera);
    }
    renderer.autoClear = prevAuto;
    renderer.setClearColor(prevClear, prevAlpha);
  }
}
