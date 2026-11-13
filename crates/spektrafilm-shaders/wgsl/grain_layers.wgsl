// GPU multilayer grain, ported from pinned SpektraLab 99c2d5c (GPL-3.0).
// Philox + exact Poisson thinning matches the reference distribution; it
// does not substitute Gaussian noise or smooth a simplified grain output.
struct Params { shape: vec4<u32>, control: vec4<u32>, values: vec4<f32>, sampling: vec4<f32>, reference:GrainReference }
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read_write> a: array<f32>;
@group(0) @binding(2) var<storage, read_write> b: array<f32>;
@group(0) @binding(3) var<storage, read> data: array<f32>;

fn mul_hi(x:u32,y:u32)->u32 {
    let x0=x&65535u; let x1=x>>16u; let y0=y&65535u; let y1=y>>16u;
    let w0=x0*y0; let t=x1*y0+(w0>>16u);
    let w1=x0*y1+(t&65535u);
    return x1*y1+(t>>16u)+(w1>>16u);
}
fn philox(counter:vec4<u32>,key0:vec2<u32>)->vec4<u32> {
    var c=counter; var key=key0;
    for(var r=0u;r<10u;r++) {
        let hi0=mul_hi(0xD2511F53u,c.x); let hi1=mul_hi(0xCD9E8D57u,c.z);
        c=vec4<u32>(hi1^c.y^key.x,0xCD9E8D57u*c.z,hi0^c.w^key.y,0xD2511F53u*c.x);
        key+=vec2<u32>(0x9E3779B9u,0xBB67AE85u);
    } return c;
}
struct Rng { ctr:vec4<u32>, key:vec2<u32>, buf:vec4<u32>, count:u32 }
fn rng_init(pixel:u32,stream:u32)->Rng {
    return Rng(vec4<u32>(pixel,stream,0u,0u),vec2<u32>(p.shape.w,0u),vec4<u32>(0u),0u);
}
fn next(r:ptr<function,Rng>)->f32 {
    if((*r).count==0u) { (*r).buf=philox((*r).ctr,(*r).key); (*r).ctr.z+=1u; (*r).count=4u; }
    let v=(*r).buf.x; (*r).buf=(*r).buf.yzwx; (*r).count-=1u;
    return (f32(v>>8u)+0.5)*(1.0/16777216.0);
}
fn loggam(x:f32)->f32 {
    if(x==1.0||x==2.0){return 0.0;}
    var n=0i; var x0=x;
    if(x<=7.0){n=i32(7.0-x);x0=x+f32(n);}
    let x2=1.0/(x0*x0);
    var g=8.417508417508418e-4;
    g=g*x2-5.952380952380952e-4;g=g*x2+7.936507936507937e-4;
    g=g*x2-2.777777777777778e-3;g=g*x2+8.333333333333333e-2;
    var result=g/x0+0.5*log(6.283185307179586)+(x0-0.5)*log(x0)-x0;
    if(x<=7.0){for(var k=1i;k<=n;k++){result-=log(x0-1.0);x0-=1.0;}}
    return result;
}
fn poisson(r:ptr<function,Rng>,mu:f32)->f32 {
    if(mu>=10.0) {
        let slam=sqrt(mu);let loglam=log(mu);let bb=0.931+2.53*slam;let aa=-0.059+0.02483*bb;
        let invalpha=1.1239+1.1328/(bb-3.4);let vr=0.9277-3.6224/(bb-2.0);
        for(var it=0u;it<64u;it++) {
            let u=next(r)-0.5;let v=next(r);let us=0.5-abs(u);
            let k=floor((2.0*aa/us+bb)*u+mu+0.43);
            if(us>=0.07&&v<=vr){return k;}
            if(k<0.0||(us<0.013&&v>us)){continue;}
            if(log(v)+log(invalpha)-log(aa/(us*us)+bb)<=-mu+k*loglam-loggam(k+1.0)){return k;}
        } return floor(mu);
    }
    let limit=exp(-mu);var prod=1.0;var count=0.0;
    for(var it=0u;it<512u;it++){prod*=next(r);if(prod>limit){count+=1.0;}else{return count;}}
    return count;
}
// Full-frame anchored continuous RNG field, same local density at each node.
// Normalize interpolation variance so scale and amount remain independent.
fn sampled_poisson(i:u32,stream:u32,mu:f32)->f32 {
    if(p.sampling.z==0.0 || p.sampling.x<=1.0){var r=rng_init(grain_index(i,p.reference),stream);return poisson(&r,mu);}
    let pos=grain_position(i,p.shape.y,p.reference)/p.sampling.x;
    let stride=select(p.shape.y,p.reference.shape.y,p.reference.shape.w!=0u);
    let cell=vec2<u32>(floor(pos));let t=fract(pos);
    let weights=vec4<f32>((1.0-t.x)*(1.0-t.y),t.x*(1.0-t.y),(1.0-t.x)*t.y,t.x*t.y);
    let ids=vec4<u32>(cell.y*stride+cell.x,cell.y*stride+cell.x+1u,(cell.y+1u)*stride+cell.x,(cell.y+1u)*stride+cell.x+1u);
    var residual=0.0;
    for(var q=0u;q<4u;q++){var r=rng_init(ids[q],stream);residual+=weights[q]*(poisson(&r,mu)-mu);}
    return max(0.0,mu+residual/sqrt(dot(weights,weights)));
}
fn reflect_index(pos:i32,len:i32)->u32 {
    let period=2*len;var q=((pos%period)+period)%period;
    if(q>=len){q=period-q-1;}return u32(q);
}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) {
    let i=gid.x+gid.y*groups.x*256u;if(i>=p.shape.x){return;}
    let op=p.control.z;
    if(op==0u||op==1u) { // all layers in place / one layer to b
        let k=p.control.x;let lp0=k*15u;
        for(var ch=0u;ch<3u;ch++) {
            var v=a[3u*i+ch];if(p.control.y!=0u){v=-v;}
            var lo=0u;var hi=k;
            loop{if(lo>=hi){break;}let mid=lo+(hi-lo)/2u;if(data[mid*3u+ch]<=v){lo=mid+1u;}else{hi=mid;}}
            var row=0u;var t=0.0;
            if(lo>=k){row=k-1u;}else if(lo>0u){row=lo-1u;t=(v-data[row*3u+ch])*data[k*3u+row*3u+ch];}
            var sum=0.0;
            for(var sl=0u;sl<3u;sl++) {
                if(op==1u&&sl!=p.control.w){continue;}
                let col=ch*3u+sl;let y0=data[k*6u+row*9u+col];
                var d=y0;if(row+1u<k){d=y0+t*(data[k*6u+(row+1u)*9u+col]-y0);}
                let offset=lp0+col*4u;let dmin=data[offset];let dmax=data[offset+1u];let n=data[offset+2u];let u=data[offset+3u];
                let probability=clamp((d+dmin)/dmax,1e-6,1.0-1e-6);
                let sat=1.0-probability*u*(1.0-1e-6);
                var stream=ch+sl*10u;if(p.values.w!=0.0){stream=sl*10u;}
                sum+=sampled_poisson(i,stream,(n/sat)*probability)*((dmax/n)*sat);
            }
            if(op==0u){a[3u*i+ch]=sum;}else{b[3u*i+ch]=sum;}
        } return;
    }
    if(op==2u||op==3u) { // reflected separable per-channel FIR
        let x=i%p.shape.y;let y=i/p.shape.y;var offset=3u;
        for(var ch=0u;ch<3u;ch++) {
            let radius=u32(data[ch]);var sum=0.0;
            for(var kk=0u;kk<=2u*radius;kk++) {
                let delta=i32(kk)-i32(radius);var pos=0u;
                if(op==2u){pos=y*p.shape.y+reflect_index(i32(x)+delta,i32(p.shape.y));}
                else{pos=reflect_index(i32(y)+delta,i32(p.shape.z))*p.shape.y+x;}
                sum+=a[3u*pos+ch]*data[offset+kk];
            } b[3u*i+ch]=sum;offset+=2u*radius+1u;
        } return;
    }
    for(var ch=0u;ch<3u;ch++) {
        let idx=3u*i+ch;
        if(op==4u) { // micro-structure, stream 100 + channel
            var rng=rng_init(grain_index(i,p.reference),100u+ch);let u1=next(&rng);let u2=next(&rng);
            let normal=sqrt(-2.0*log(u1))*cos(6.283185307179586*u2);
            b[idx]=exp(p.values.x+p.values.y*normal);
        }else if(op==5u){a[idx]+=b[idx];}
        else if(op==6u){if(p.values.w!=0.0){a[idx]*=b[idx];}a[idx]-=p.values[ch];}
        else if(op==7u){b[idx]=a[idx];}
        else if(op==8u){b[idx]=max(0.0,a[idx]+p.sampling.y*(b[idx]-a[idx]));}
    }
}
