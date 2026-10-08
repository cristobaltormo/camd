#include "settings.h"

#include <Preferences.h>
#include <WiFi.h>

Settings settings;

static const char *NAMESPACE = "cam";

static String defaultHostname() {
  uint8_t mac[6];
  WiFi.macAddress(mac);
  char name[20];
  snprintf(name, sizeof(name), "camd-%02x%02x%02x", mac[3], mac[4], mac[5]);
  return name;
}

void Settings::load() {
  Preferences prefs;
  prefs.begin(NAMESPACE, true);
  ssid = prefs.getString("ssid", "");
  password = prefs.getString("pass", "");
  token = prefs.getString("token", "");
  hostname = prefs.getString("hostname", "");
  prefs.end();
  if (hostname.isEmpty()) hostname = defaultHostname();
}

void Settings::save(const char *key, const String &value) {
  String name(key);
  if (name == "ssid") ssid = value;
  else if (name == "pass") password = value;
  else if (name == "token") token = value;
  else if (name == "hostname") hostname = value;
  Preferences prefs;
  prefs.begin(NAMESPACE, false);
  prefs.putString(key, value);
  prefs.end();
}

void Settings::wipe() {
  Preferences prefs;
  prefs.begin(NAMESPACE, false);
  prefs.clear();
  prefs.end();
}
