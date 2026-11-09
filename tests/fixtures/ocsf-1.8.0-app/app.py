import subprocess
import urllib.request


def github_zen():
    # @openshell egress api.github.com:443 reason="allowed host" access=read-only
    return urllib.request.urlopen("https://api.github.com/zen").read()


def httpbin_get():
    # @openshell egress httpbin.org:443 reason="L7 path rule" allow=GET:/get
    return urllib.request.urlopen("https://httpbin.org/get").read()


def httpbin_post():
    # @openshell ignore kind=net reason="host is declared above; the L7 rule denies this request"
    return urllib.request.urlopen("https://httpbin.org/post", data=b"x").read()


def curl_icanhazip():
    # @openshell egress icanhazip.com:443 reason="rule bound to curl only" access=read-only bin=/usr/bin/curl
    return subprocess.run(["curl", "-sS", "https://icanhazip.com"], check=True)


def example_com():
    # Deliberately unannotated: the sandbox blocks it.
    return subprocess.run(["curl", "-sS", "https://example.com"], check=True)


def todos():
    # @openshell egress jsonplaceholder.typicode.com:443 reason="never called in the captured run" allow=GET:/todos/1
    return urllib.request.urlopen("https://jsonplaceholder.typicode.com/todos/1").read()
