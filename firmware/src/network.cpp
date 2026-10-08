#include "network.h"

#include <DNSServer.h>
#include <ESPmDNS.h>
#include <WiFi.h>

#include "settings.h"

static const uint32_t PORTAL_AFTER_MS = 2UL * 60UL * 1000UL;
static const uint32_t RESTART_AFTER_MS = 10UL * 60UL * 1000UL;
static const uint8_t DNS_PORT = 53;

static DNSServer dns;
static bool portalActive = false;
static bool mdnsStarted = false;
static uint32_t lastConnected = 0;
static String scanResult = "[]";

static String portalName() { return "camd-setup-" + settings.hostname.substring(settings.hostname.length() - 4); }

static void scanNetworks() {
  int count = WiFi.scanNetworks();
  String json = "[";
  for (int i = 0; i < count; i++) {
    if (WiFi.SSID(i).isEmpty()) continue;
    if (json.length() > 1) json += ",";
    String name = WiFi.SSID(i);
    name.replace("\\", "\\\\");
    name.replace("\"", "\\\"");
    json += "{\"ssid\":\"" + name + "\",\"rssi\":" + String(WiFi.RSSI(i)) + ",\"open\":" + (WiFi.encryptionType(i) == WIFI_AUTH_OPEN ? "true" : "false") + "}";
  }
  json += "]";
  WiFi.scanDelete();
  scanResult = json;
}

static void stopPortal() {
  if (!portalActive) return;
  dns.stop();
  WiFi.softAPdisconnect(true);
  WiFi.mode(WIFI_STA);
  portalActive = false;
}

static void startMdns() {
  if (mdnsStarted || !MDNS.begin(settings.hostname.c_str())) return;
  MDNS.addService("http", "tcp", 80);
  mdnsStarted = true;
}

void networkStartPortal() {
  if (portalActive) return;
  WiFi.mode(WIFI_AP_STA);
  scanNetworks();
  WiFi.softAP(portalName().c_str());
  dns.start(DNS_PORT, "*", WiFi.softAPIP());
  portalActive = true;
  Serial.printf("setup portal: join wifi '%s' and open http://%s\n", portalName().c_str(), WiFi.softAPIP().toString().c_str());
}

bool networkPortalActive() { return portalActive; }

String networkScanJson() { return scanResult; }

void networkBegin() {
  WiFi.mode(WIFI_STA);
  WiFi.setHostname(settings.hostname.c_str());
  WiFi.setSleep(false);
  WiFi.setAutoReconnect(true);
  lastConnected = millis();
  if (!settings.wifiConfigured()) {
    Serial.println("wifi not configured");
    networkStartPortal();
    return;
  }
  WiFi.begin(settings.ssid.c_str(), settings.password.c_str());
}

void networkLoop() {
  if (WiFi.status() == WL_CONNECTED) {
    lastConnected = millis();
    startMdns();
    if (portalActive && settings.wifiConfigured()) stopPortal();
  } else if (settings.wifiConfigured()) {
    uint32_t offline = millis() - lastConnected;
    if (!portalActive && offline > PORTAL_AFTER_MS) networkStartPortal();
    if (offline > RESTART_AFTER_MS) ESP.restart();
  }
  if (portalActive) dns.processNextRequest();
}
