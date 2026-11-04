// @openshell egress *.crates.io:443 reason="crate index and downloads" access=read-only rule=crates
fn main() {
    // A string is not a comment.
    let hidden = "// @openshell egress not-a-directive.test:443 reason=\"inside a string\"";
    let _ = hidden;
}
