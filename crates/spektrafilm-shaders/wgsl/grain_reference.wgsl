// Optional source-anchored stochastic coordinates for film grain.
// Zero-initialized reference preserves the original per-render RNG byte for byte.
struct GrainReference { row_x:vec4<f32>, row_y:vec4<f32>, shape:vec4<u32> }
fn grain_position(i:u32, width:u32, r:GrainReference)->vec2<f32> {
    let p=vec2<f32>(f32(i%width),f32(i/width));
    if(r.shape.w==0u){return p;}
    let edge=vec3<f32>(p+vec2<f32>(0.5),1.0);
    return clamp(vec2<f32>(dot(r.row_x.xyz,edge),dot(r.row_y.xyz,edge))-vec2<f32>(0.5),
        vec2<f32>(0.0),vec2<f32>(f32(r.shape.y-1u),f32(r.shape.z-1u)));
}
fn grain_index(i:u32,r:GrainReference)->u32 {
    if(r.shape.w==0u){return i;}
    let p=vec2<u32>(floor(grain_position(i,r.shape.x,r)+vec2<f32>(0.5)));
    return p.y*r.shape.y+p.x;
}
