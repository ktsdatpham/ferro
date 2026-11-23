// Ferro viewport shader (working quality, D-48 / Q-G2).

struct Globals {
    view_proj: mat4x4<f32>,
    light_dir: vec4<f32>,
    eye: vec4<f32>,
    // Section plane: fragments with dot(n, p) > w are cut away.
    clip: vec4<f32>,
    // x = 1: flat white faces (hidden line).
    style: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) world: vec3<f32>,
};

fn cut(p: vec3<f32>) -> bool {
    return dot(g.clip.xyz, p) > g.clip.w;
}

@vertex
fn vs_mesh(v: VIn) -> VOut {
    var o: VOut;
    o.clip = g.view_proj * vec4<f32>(v.pos, 1.0);
    o.normal = v.normal;
    o.color = v.color;
    o.world = v.pos;
    return o;
}

@fragment
fn fs_mesh(i: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    if (cut(i.world)) { discard; }
    if (g.style.x > 0.5) { return vec4<f32>(1.0, 1.0, 1.0, i.color.a); }
    var n = normalize(i.normal);
    if (!front) { n = -n; }
    let l = normalize(g.light_dir.xyz);
    let diffuse = max(dot(n, l), 0.0);
    // Sky / ground hemisphere fill so vertical faces read clearly.
    let hemi = 0.5 + 0.5 * n.z;
    let light = 0.42 + 0.48 * diffuse + 0.12 * hemi;
    var c = i.color.rgb * light;
    if (!front) { c = c * vec3<f32>(0.75, 0.78, 0.95); } // back faces tinted, like SketchUp
    return vec4<f32>(c, i.color.a);
}

@vertex
fn vs_line(v: VIn) -> VOut {
    var o: VOut;
    // Edges (normal.x = 1) are pulled 0.15 % towards the camera so they win
    // against their own faces on every GPU; grid lines (0) are not.
    let pull = 1.0 - 0.0015 * v.normal.x;
    let p = g.eye.xyz + (v.pos - g.eye.xyz) * pull;
    o.clip = g.view_proj * vec4<f32>(p, 1.0);
    o.normal = v.normal;
    o.color = v.color;
    o.world = v.pos;
    return o;
}

@fragment
fn fs_line(i: VOut) -> @location(0) vec4<f32> {
    if (cut(i.world)) { discard; }
    return i.color;
}
