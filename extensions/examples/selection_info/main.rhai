// Selection Info: an observer that shows counts and the area of the selection in
// the status bar (at most every 300 ms), and Extensions > Selection Info > Show
// Selection Info for the same text in a message.

on_selection_changed("update_status", 300);

register_command(#{
    id: "show",
    label: "Show Selection Info",
    menu: "Extensions > Selection Info",
    callback: "show_message",
});

/// "3 selected: 1 face (12.5 m²), 2 edges, 0 groups/components"
fn summary() {
    let sel = selection();
    let faces = 0;
    let edges = 0;
    let groups = 0;
    let other = 0;
    let area = 0.0;
    for e in sel {
        switch e.kind {
            "face" => { faces += 1; area += e.area; }
            "edge" => { edges += 1; }
            "group" | "component" => { groups += 1; }
            _ => { other += 1; }
        }
    }
    let face_word = if faces == 1 { "face" } else { "faces" };
    let edge_word = if edges == 1 { "edge" } else { "edges" };
    let text = `${sel.len()} selected: ${faces} ${face_word} (${format_area(area)}), ${edges} ${edge_word}, ${groups} groups/components`;
    if other > 0 {
        text += `, ${other} other`;
    }
    text
}

fn update_status() {
    if selection().is_empty() {
        status("");
    } else {
        status(summary());
    }
}

fn show_message() {
    if selection().is_empty() {
        message("Nothing is selected.");
    } else {
        message(summary());
    }
}
