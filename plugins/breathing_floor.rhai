//! name: Breathing Floor
//! version: 1.0
//! author: DMXpress
//! description: A slow sine breath over every dimmer in the rig, spread across the stage from left to right. Runs as its own layer under the programmer.
//! blend: highest

fn controls() {
    [
        #{ kind: "label",    id: "hint",  text: "Breathes every dimmer, fanned across the stage." },
        #{ kind: "slider",   id: "rate",  label: "Rate (cycles/min)", min: 2.0,  max: 60.0, value: 12.0 },
        #{ kind: "slider",   id: "depth", label: "Depth",             min: 0.0,  max: 1.0,  value: 0.6 },
        #{ kind: "slider",   id: "floor", label: "Floor level",       min: 0.0,  max: 1.0,  value: 0.05 },
        #{ kind: "checkbox", id: "fan",   label: "Fan across the stage", value: true },
    ]
}

fn frame(t, rig, state) {
    let rate = state.rate;
    let depth = state.depth;
    let floor = state.floor;
    let out = [];
    if rig.len() == 0 { return out; }

    // Stage extent, for the left-to-right fan.
    let min_x = 1e9; let max_x = -1e9;
    for f in rig {
        if f.x < min_x { min_x = f.x; }
        if f.x > max_x { max_x = f.x; }
    }
    let span = max_x - min_x;

    let w = rate / 60.0 * 2.0 * 3.14159265;
    for f in rig {
        if f.dimmer < 1 { continue; }
        let phase = 0.0;
        if state.fan && span > 0.001 {
            phase = (f.x - min_x) / span * 3.14159265;
        }
        // 0..1 breath, shaped so the swell is soft.
        let breath = (1.0 + sin(t * w + phase)) / 2.0;
        let level = floor + (1.0 - floor) * breath * depth;
        out.push([f.dimmer, (level * 255.0).to_int()]);
    }
    out
}
