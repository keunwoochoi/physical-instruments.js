// Render our model at the SAME notes/velocities as a reference set, to WAV, so the two
// go through the IDENTICAL analyzer. Anything else compares two different measurements.
import fs from 'node:fs';
const SR=48000,Q=128;
const [,,wasmPath,inst,outDir,spec] = process.argv;
const cases = JSON.parse(spec); // [{midi, vel, name}]
fs.mkdirSync(outDir,{recursive:true});
const {instance}=await WebAssembly.instantiate(fs.readFileSync(wasmPath),{});
const x=instance.exports;
function wav(b){const n=b.length;const o=Buffer.alloc(44+n*2);
 o.write('RIFF',0);o.writeUInt32LE(36+n*2,4);o.write('WAVEfmt ',8);o.writeUInt32LE(16,16);
 o.writeUInt16LE(1,20);o.writeUInt16LE(1,22);o.writeUInt32LE(SR,24);o.writeUInt32LE(SR*2,28);
 o.writeUInt16LE(2,32);o.writeUInt16LE(16,34);o.write('data',36);o.writeUInt32LE(n*2,40);
 for(let i=0;i<n;i++)o.writeInt16LE(Math.max(-32768,Math.min(32767,(b[i]*32767)|0)),44+i*2);
 return o;}
for(const c of cases){
  const p=x.ij_engine_new(SR);x.ij_set_track(p,0,+inst,1.0,0.0);
  const dur=3.0, n=Math.ceil(dur*SR/Q);
  const b=new Float32Array(n*Q);const lp=x.ij_out_l(p);
  x.ij_note_on(p,0,c.midi,c.vel);
  for(let q=0;q<n;q++){
    if(q*Q>=2.5*SR&&(q-1)*Q<2.5*SR)x.ij_note_off(p,0,c.midi);
    x.ij_process(p,Q);b.set(new Float32Array(x.memory.buffer,lp,Q),q*Q);
  }
  fs.writeFileSync(`${outDir}/${c.name}.wav`, wav(b));
}
console.log(`  rendered ${cases.length} -> ${outDir}`);
