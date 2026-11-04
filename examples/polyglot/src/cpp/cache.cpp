// @openshell egress objects.example.com:443 reason="blob store" access=read-only
// @openshell fs write /workspace/blobs reason="cached blobs"
int cache_put(void) {
    return 0;
}
