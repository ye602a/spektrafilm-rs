// Exact f32 -> round(f64(value) * 65535) using the f32 significand.
// A plain f32 multiply can round across half-code boundaries. Integer arithmetic
// preserves the existing CPU result without requiring shader f64 support.
struct Quantize { channels:u32, row_stride:u32, unused0:u32, unused1:u32 }
@group(0) @binding(0) var<storage,read> source:array<f32>;
@group(0) @binding(1) var<storage,read_write> packed:array<u32>;
@group(0) @binding(2) var<uniform> parameters:Quantize;

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

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id:vec3<u32>) {
    let pair=id.y*parameters.row_stride+id.x;
    let first=pair*2u;
    if first>=parameters.channels {return;}
    var second=0u;
    if first+1u<parameters.channels {second=code(source[first+1u]);}
    packed[pair]=code(source[first])|(second<<16u);
}
