fn code(value:f32)->u32 {
    let bits=bitcast<u32>(value);
    if (bits & 0x80000000u)!=0u || bits>0x7f800000u {return 0u;}
    if bits>=0x3f800000u {return 65535u;}
    let exponent=bits>>23u;
    if exponent<110u {return 0u;}
    let mantissa=(bits & 0x7fffffu)|0x800000u;
    // mantissa * 65535 = (mantissa << 16) - mantissa, as two words.
    let shifted=mantissa<<16u;
    let low=shifted-mantissa;
    var high=(mantissa>>16u)-select(0u,1u,shifted<mantissa);
    let shift=150u-exponent;
    if shift<=32u {
        let rounded=low+(1u<<(shift-1u));
        high+=select(0u,1u,rounded<low);
        if shift==32u {return high;}
        return (rounded>>shift)|(high<<(32u-shift));
    }
    return (high+(1u<<(shift-33u)))>>(shift-32u);
}


struct View {source:vec4<u32>,crop:vec4<u32>,output:vec4<u32>,offset:vec4<u32>,length:vec4<u32>}
@group(0) @binding(0) var<storage,read> input:array<f32>;
@group(0) @binding(1) var<storage,read> nodes:array<vec4<f32>>;
@group(0) @binding(2) var<uniform> view:View;
@group(0) @binding(3) var display:texture_storage_2d<rgba8unorm,write>;
@group(0) @binding(4) var precision_output:texture_storage_2d<rgba16uint,write>;
@group(0) @binding(5) var<storage,read> finish:array<u32>;
fn exposure(p:f32,c:u32)->f32 {
    let offset=view.offset[c];let n=view.length[c];var lo=0u;var hi=n;
    loop {if lo>=hi {break;}let mid=(lo+hi)/2u;let q=nodes[offset+mid];
        if q.x>p || (q.x==p && q.y>=0.0) {lo=mid+1u;}else{hi=mid;}}
    var value=0.0;
    if lo==n {value=nodes[offset+n-1u].z;}
    else if lo>0u {let a=nodes[offset+lo-1u];let b=nodes[offset+lo];
        let t=((a.x-p)+a.y)*a.w;value=a.z*(1.0-t)+b.z*t;}
    return (0.18/256.0)*sinh(value);
}
fn encode(x:f32)->f32 {if x<=0.0031308 {return 12.92*x;}return 1.055*pow(x,1.0/2.4)-0.055;}
fn positive(p:vec3<f32>)->vec3<f32> {
    let ap=vec3<f32>(exposure(p.r,0u),exposure(p.g,1u),exposure(p.b,2u));
    var rgb=vec3<f32>(dot(ap,vec3<f32>(2.5216494298433036,-1.13688855422226,-0.3849175931944449)),
        dot(ap,vec3<f32>(-0.27521355124402597,1.3697051510263254,-0.09439245077651998)),
        dot(ap,vec3<f32>(-0.015925010090464285,-0.14780636811079967,1.1638058159424312)));
    let y=max(dot(rgb,vec3<f32>(0.2126,0.7152,0.0722)),0.0);
    let t=clamp(y/(0.18/256.0),0.0,1.0);rgb=vec3<f32>(y)+(rgb-vec3<f32>(y))*(t*t*(3.0-2.0*t));
    var out=y;if y>0.75 {out=0.75+0.25*(1.0-exp(-(y-0.75)/0.25));}
    rgb*=out/max(y,1e-12);
    let low=min(rgb.r,min(rgb.g,rgb.b));let high=max(rgb.r,max(rgb.g,rgb.b));
    let f=max(0.0,min(1.0,min(out/max(out-low,1e-12),(1.0-out)/max(high-out,1e-12))));
    rgb=vec3<f32>(out)+f*(rgb-vec3<f32>(out));
    return clamp(vec3<f32>(encode(rgb.r),encode(rgb.g),encode(rgb.b)),vec3<f32>(0),vec3<f32>(1));
}
fn pixel(p:vec2<u32>)->vec3<u32> {
    let i=((p.y+view.crop.y)*view.source.x+p.x+view.crop.x)*3u;
    var rgb=vec3<f32>(input[i],input[i+1u],input[i+2u]);if view.source.z!=0u {rgb=positive(rgb);}
    var result=vec3<u32>(code(rgb.r),code(rgb.g),code(rgb.b));
    if view.source.w!=0u {result=vec3<u32>(finish[result.r],finish[result.g],finish[result.b]);}return result;
}
@compute @workgroup_size(16,16)
fn main(@builtin(global_invocation_id) id:vec3<u32>) {
    if any(id.xy>=view.output.xy) {return;}
    var rgb:vec3<u32>;
    if all(view.output.xy==view.crop.zw) {rgb=pixel(id.xy);}else {
        let ratio=vec2<f32>(view.crop.zw)/vec2<f32>(view.output.xy);let start=vec2<f32>(id.xy)*ratio;
        let first=min(vec2<u32>(ceil(start)),view.crop.zw-vec2<u32>(1));let last=min(vec2<u32>(ceil(start+ratio)),view.crop.zw);
        var total=vec3<f32>(0);
        for(var y=first.y;y<last.y;y++) {var row=vec3<u32>(0);for(var x=first.x;x<last.x;x++){row+=pixel(vec2<u32>(x,y));}total+=vec3<f32>(row);}
        rgb=vec3<u32>(floor(total/f32((last.x-first.x)*(last.y-first.y))+0.5));
    }
    textureStore(precision_output,id.xy,vec4<u32>(rgb,65535u));
    textureStore(display,id.xy,vec4<f32>(vec3<f32>((rgb+vec3<u32>(128u))/257u)/255.0,1.0));
}