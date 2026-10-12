// Stair Maker: Extensions > Stair Maker > Make Stair... (and a toolbar button).
// A dialog asks for the width, rise, run and step count; the stair is drawn as a
// side profile, pushed out to its width, inside a new group (one undo step).

register_command(#{
    id: "make",
    label: "Make Stair...",
    menu: "Extensions > Stair Maker",
    toolbar: true,
    tooltip: "Stair Maker",
    icon: "icon.png",
    callback: "ask",
});

/// The last values used (inches), or the defaults.
fn last_values() {
    let last = get_state("last");
    if last == () {
        last = #{ width: mm(900.0), rise: mm(175.0), run: mm(280.0), count: 10 };
    }
    last
}

fn ask() {
    let v = last_values();
    input("Stair Maker", [
        #{ key: "width", label: "Width", type: "length", value: v.width },
        #{ key: "rise", label: "Rise per step", type: "length", value: v.rise },
        #{ key: "run", label: "Run (tread depth)", type: "length", value: v.run },
        #{ key: "count", label: "Steps", type: "number", value: v.count },
    ], "make_stair");
}

/// The side profile in the red-blue plane: up one rise, along one run, per step.
fn profile(rise, run, n) {
    let pts = [[0.0, 0.0, 0.0]];
    for i in 0..n {
        let z = (i + 1) * rise;
        pts.push([i * run, 0.0, z]);
        pts.push([(i + 1) * run, 0.0, z]);
    }
    pts.push([n * run, 0.0, 0.0]);
    pts
}

/// Makes the stair from the dialog's values (also callable from the console:
/// `make_stair(#{ width: 36.0, rise: 7.0, run: 11.0, count: 5 })`).
fn make_stair(v) {
    let n = v.count.to_int();
    if n < 1 || n > 200 {
        throw "Steps must be between 1 and 200.";
    }
    if v.width <= 0.0 || v.rise <= 0.0 || v.run <= 0.0 {
        throw "Width, rise and run must be more than zero.";
    }
    set_state("last", v);
    let g = new_group("Stair");
    edit_group(g);
    let face = add_face(profile(v.rise, v.run, n));
    if face == () {
        throw "Could not make the stair's side face.";
    }
    // Push along +green whichever way the face looks.
    let d = if face.normal[1] > 0.0 { v.width } else { -v.width };
    push_pull(face, d);
    end_group();
    select([g]);
    status(`Stair: ${n} steps, ${format_length(n * v.rise)} high, ${format_length(n * v.run)} long`);
}
