"""Wheel cache.

A docstring is a string, not a comment, so this line is ignored:
# @openshell egress not-a-directive.test:443 reason="inside a docstring"
"""

import os

# @openshell egress pypi.org:443 reason="package index" access=read-only scope=file
# @openshell egress files.pythonhosted.org:443 reason="wheel downloads" access=read-only scope=file
# @openshell fs read /etc/ssl/certs reason="CA bundle"
# @openshell fs write /workspace/cache reason="downloaded wheels"


def fetch_index():
    import requests

    return requests.get("https://pypi.org/simple/requests/")


def fetch_wheel():
    import requests

    return requests.get(
        "https://files.pythonhosted.org/packages/source/r/requests/requests-2.32.3.tar.gz"
    )


def save(body: bytes) -> None:
    with open("/workspace/cache/requests.tar.gz", "wb") as handle:
        handle.write(body)
    os.makedirs("/workspace/cache/unpacked", exist_ok=True)
