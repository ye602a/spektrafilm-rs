// Display extraction only: input already contains the full physical result.
// Shrinking mirrors the RGB16 area blocks used by image::thumbnail. At 100%
// copy the original f32 exactly; CPU quantization then retains exact codes.
struct Region { source:vec4<u32>, crop:vec4<u32>, output:vec4<u32> }
@group(0) @binding(0) var<storage,read> input:array<f32>;
@group(0) @binding(1) var<storage,read_write> result:array<f32>;
@group(0) @binding(2) var<uniform> region:Region;

fn pixel(p:vec2<u32>)->vec3<f32> {
    let i=((p.y+region.crop.y)*region.source.x+p.x+region.crop.x)*3u;
    return vec3<f32>(input[i],input[i+1u],input[i+2u]);
}
@compute @workgroup_size(16,16)
fn main(@builtin(global_invocation_id) id:vec3<u32>) {
    if any(id.xy>=region.output.xy) {return;}
    var color:vec3<f32>;
    if all(region.output.xy==region.crop.zw) {
        color=pixel(id.xy);
    } else {
        let ratio=vec2<f32>(region.crop.zw)/vec2<f32>(region.output.xy);
        let start=vec2<f32>(id.xy)*ratio;
        let first=min(vec2<u32>(ceil(start)),region.crop.zw-vec2<u32>(1));
        let last=min(vec2<u32>(ceil(start+ratio)),region.crop.zw);
        var total=vec3<f32>(0);
        for(var y=first.y;y<last.y;y++) {
            var row=vec3<u32>(0);
            for(var x=first.x;x<last.x;x++) {
                row+=vec3<u32>(floor(clamp(pixel(vec2<u32>(x,y)),vec3<f32>(0),vec3<f32>(1))*65535.0+0.5));
            }
            total+=vec3<f32>(row);
        }
        let count=f32((last.x-first.x)*(last.y-first.y));
        color=floor(total/count+0.5)/65535.0;
    }
    let i=(id.y*region.output.x+id.x)*3u;
    result[i]=color.r;result[i+1u]=color.g;result[i+2u]=color.b;
}
