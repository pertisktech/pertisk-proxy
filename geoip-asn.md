## install
```sh
mkdir -p /var/lib/pertisk-proxy/geoip/
cd tmp
wget https://iptoasn.com/data/ip2asn-combined.tsv.gz
gzip -d  ip2asn-combined.tsv.gz
cp ip2asn-combined.tsv /var/lib/pertisk-proxy/geoip/

wget https://cdn.jsdelivr.net/npm/geolite2-country/GeoLite2-Country.mmdb.gz
gzip -d GeoLite2-Country.mmdb.gz
cp GeoLite2-Country.mmdb /var/lib/pertisk-proxy/geoip/
ls -la /var/lib/pertisk-proxy/geoip/
```