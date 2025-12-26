#!/bin/bash
# Generate self-signed TLS certificates for development

set -e

echo "🔒 Generating TLS Certificates..."

# Create certs directory
mkdir -p certs

# Generate private key + certificate (valid 1 year)
openssl req -x509 -newkey rsa:4096 -nodes \
  -keyout certs/key.pem \
  -out certs/cert.pem \
  -days 365 \
  -subj "/CN=localhost" \
  -addext "subjectAltName=DNS:localhost,IP:127.0.0.1"

echo "✅ Certificates generated:"
echo "   - certs/key.pem (private key)"
echo "   - certs/cert.pem (certificate)"
echo ""
echo "⚠️  WARNING: Self-signed cert only for dev!"
echo "   Production: Use Let's Encrypt"