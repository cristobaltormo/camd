#include <Arduino.h>
#include <ArduinoOTA.h>
#include <esp_task_wdt.h>

#include "camera.h"
#include "console.h"
#include "network.h"
#include "pins.h"
#include "settings.h"
#include "webserver.h"

static const uint32_t WATCHDOG_S = 30;
static const uint32_t CAMERA_RETRY_DELAY_MS = 2000;

static void startOta() {
  if (settings.token.isEmpty()) return;
  ArduinoOTA.setHostname(settings.hostname.c_str());
  ArduinoOTA.setPassword(settings.token.c_str());
  ArduinoOTA.onStart([]() { esp_task_wdt_delete(nullptr); });
  ArduinoOTA.onError([](ota_error_t) { esp_task_wdt_add(nullptr); });
  ArduinoOTA.begin();
}

void setup() {
  Serial.begin(115200);
  delay(300);
  pinMode(FLASH_LED_GPIO, OUTPUT);
  digitalWrite(FLASH_LED_GPIO, LOW);

  settings.load();
  Serial.printf("\ncamd camera fw %s, host %s\n", FW_VERSION, settings.hostname.c_str());

  if (!cameraStart()) {
    delay(CAMERA_RETRY_DELAY_MS);
    ESP.restart();
  }

  networkBegin();
  webserverStart();
  startOta();
  esp_task_wdt_init(WATCHDOG_S, true);
  esp_task_wdt_add(nullptr);
}

void loop() {
  esp_task_wdt_reset();
  consoleLoop();
  networkLoop();
  webserverLoop();
  ArduinoOTA.handle();
  if (cameraStuck()) {
    Serial.println("camera stopped delivering frames, restarting");
    ESP.restart();
  }
  delay(20);
}
