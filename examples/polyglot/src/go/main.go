package main

import (
	"io"
	"net/http"
	"os"
)

// @openshell bin /opt/desk/server
// @openshell process user=sandbox group=sandbox
// @openshell egress proxy.golang.org:443 reason="module proxy" access=read-only
// @openshell fs write /workspace/data reason="module list cache"
func main() {
	resp, err := http.Get("https://proxy.golang.org/github.com/golang/mod/@v/list")
	if err != nil {
		return
	}
	defer resp.Body.Close()
	body, _ := io.ReadAll(resp.Body)
	_ = os.WriteFile("/workspace/data/list.txt", body, 0o644)
}
