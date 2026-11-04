# @openshell egress rubygems.org:443 reason="gem metadata" access=read-only
require "net/http"
Net::HTTP.get(URI("https://rubygems.org/api/v1/gems/rake.json"))
