#include "console.h"

#include <Arduino.h>
#include <WiFi.h>

#include "camera.h"
#include "network.h"
#include "settings.h"

static const size_t MAX_LINE = 160;

static void printStatus() {
  Serial.printf("fw %s | host %s | wifi %s | ip %s | rssi %d | ssid '%s' | token %s | psram %u | reset %d\n", FW_VERSION,
                settings.hostname.c_str(), WiFi.status() == WL_CONNECTED ? "up" : "down", WiFi.localIP().toString().c_str(),
                WiFi.RSSI(), settings.ssid.c_str(), settings.token.isEmpty() ? "unset" : "set", ESP.getPsramSize(),
                (int)esp_reset_reason());
}

static void scan() {
  int count = WiFi.scanNetworks();
  Serial.printf("%d networks\n", count);
  for (int i = 0; i < count; i++) {
    Serial.printf("  %-32s ch%-2d %4d dBm  %s\n", WiFi.SSID(i).c_str(), WiFi.channel(i), WiFi.RSSI(i),
                  WiFi.encryptionType(i) == WIFI_AUTH_OPEN ? "open" : "secured");
  }
  WiFi.scanDelete();
}

static void handleLine(String line) {
  line.trim();
  if (line.isEmpty()) return;
  int space = line.indexOf(' ');
  String command = space < 0 ? line : line.substring(0, space);
  String argument = space < 0 ? "" : line.substring(space + 1);

  if (command == "scan") {
    scan();
  } else if (command == "ssid" || command == "pass" || command == "token" || command == "hostname") {
    settings.save(command.c_str(), argument);
    Serial.printf("%s saved\n", command.c_str());
  } else if (command == "status") {
    printStatus();
  } else if (command == "portal") {
    networkStartPortal();
  } else if (command == "reboot") {
    ESP.restart();
  } else if (command == "wipe") {
    settings.wipe();
    Serial.println("settings wiped");
  } else {
    Serial.println("commands: scan ssid pass token hostname status portal reboot wipe");
  }
}

void consoleLoop() {
  static String line;
  while (Serial.available()) {
    char c = Serial.read();
    if (c == '\n' || c == '\r') {
      handleLine(line);
      line = "";
    } else if (line.length() < MAX_LINE) {
      line += c;
    }
  }
}
