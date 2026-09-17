//! name: Strobe Burst
//! version: 1.0
//! author: DMXpress
//! description: A one-shot white flash over the whole rig that decays back out - mash the button with the kick drum. Adds on top of whatever is running.
//! blend: add

fn controls() {
    [
        #{ kind: "label",  id: "hint",  text: "Fire a decaying white burst over the rig." },
        #{ kind: "button", id: "burst", label: "BURST" },
        #{ kind: "slider", id: "decay", label: "Decay (s)", min: 0.1, max: 2.0, value: 0.5 },
    ]
}

fn button(id, t, state) {
    if id == "burst" {
        state.fired_at = t;
    }
    state
}

fn frame(t, rig, state) {
    let out = [];
    if !("fired_at" in state) { return out; }
    let decay = state.decay;
    let age = t - state.fired_at;
    if age < 0.0 || age > decay { return out; }

    // 1 at the hit, easing out to 0.
    let k = 1.0 - age / decay;
    let v = (255.0 * k * k).to_int();
    for f in rig {
        if f.dimmer > 0 { out.push([f.dimmer, v]); }
        if f.red   > 0 { out.push([f.red,   v]); }
        if f.green > 0 { out.push([f.green, v]); }
        if f.blue  > 0 { out.push([f.blue,  v]); }
    }
    out
}
