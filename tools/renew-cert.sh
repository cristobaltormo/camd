#!/usr/bin/env bash
# Renew the cam.cristobal.cc certificate (DNS-01 via Cloudflare) and push it to LXC 131.
# Runs weekly from cron on the Proxmox host; lego only renews when fewer than 30 days remain.
set -uo pipefail
export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

LEGO_DIR=/var/lib/lego
CERT=$LEGO_DIR/.lego/certificates/cam.cristobal.cc
ID=131
DEST=/etc/ssl/habitacion-cam

fail() {
  /root/cerebro/bin/avisar.sh "Certificado de la camara" "$1" >/dev/null 2>&1 || true
  echo "ERROR: $1" >&2
  exit 1
}

set -a; . /root/cerebro/secrets.env; set +a
export CLOUDFLARE_DNS_API_TOKEN="$CLOUDFLARE_API_TOKEN"

cd "$LEGO_DIR" || fail "no existe $LEGO_DIR"
/usr/local/bin/lego-official run -a -m info@cristobaltormo.com -d cam.cristobal.cc --dns cloudflare \
  --dns.resolvers 1.1.1.1:53,8.8.8.8:53 --renew-days 30 --no-random-sleep >/tmp/renew-cert.log 2>&1 || fail "lego no pudo renovar (ver /tmp/renew-cert.log)"

new=$(sha256sum "$CERT.crt" | cut -d' ' -f1)
old=$(pct exec $ID -- sha256sum $DEST/fullchain.pem 2>/dev/null | cut -d' ' -f1)
[ "$new" = "$old" ] && exit 0

pct exec $ID -- mkdir -p $DEST
pct push $ID "$CERT.crt" $DEST/fullchain.pem --perms 0644
pct push $ID "$CERT.key" $DEST/privkey.pem --perms 0600
pct exec $ID -- nginx -t >/dev/null 2>&1 || fail "nginx rechaza el certificado nuevo"
pct exec $ID -- systemctl reload nginx || fail "no se pudo recargar nginx"
echo "certificado actualizado"
