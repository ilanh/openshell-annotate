// @openshell egress db.internal.example:5432 reason="postgres, relayed without TLS inspection" protocol=tcp tls=skip
const char *hidden = "// @openshell egress not-a-directive.test:443 reason=\"inside a string\"";

int connect_db(void) {
    return hidden != 0;
}
