#pragma once

#include <Arduino.h>

struct Settings {
  String ssid;
  String password;
  String token;
  String hostname;

  void load();
  void save(const char *key, const String &value);
  void wipe();
  bool wifiConfigured() const { return !ssid.isEmpty(); }
};

extern Settings settings;
