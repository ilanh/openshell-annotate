<?php
// PHP also has hash comments. This language is read as // and /* */, so the next line is not a directive.
# @openshell egress not-a-directive.test:443 reason="hash comments are invisible here"

// @openshell egress repo.packagist.org:443 reason="package metadata" protocol=graphql allow="query"
function packages(string $name): void
{
    echo $name;
}
