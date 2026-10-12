#include <Arduino.h>
#include <ArduinoOTA.h>
#include <Preferences.h>
#include <WiFi.h>
#include <esp_camera.h>
#include <esp_http_server.h>

#include "camera_pins.h"

static Preferences prefs;
static String cfgSsid, cfgPass, cfgToken;
static httpd_handle_t apiServer = nullptr;
static httpd_handle_t streamServer = nullptr;
static uint32_t lastWifiOk = 0;
static bool standby = false;
static const uint32_t WIFI_RESTART_AFTER_MS = 5UL * 60UL * 1000UL;

static void loadConfig() {
  prefs.begin("cam", false);
  cfgSsid = prefs.getString("ssid", "");
  cfgPass = prefs.getString("pass", "");
  cfgToken = prefs.getString("token", "");
  prefs.end();
}

static void saveKey(const char *key, const String &value) {
  prefs.begin("cam", false);
  prefs.putString(key, value);
  prefs.end();
}

static bool initCamera() {
  camera_config_t c = {};
  c.ledc_channel = LEDC_CHANNEL_0;
  c.ledc_timer = LEDC_TIMER_0;
  c.pin_d0 = Y2_GPIO_NUM;
  c.pin_d1 = Y3_GPIO_NUM;
  c.pin_d2 = Y4_GPIO_NUM;
  c.pin_d3 = Y5_GPIO_NUM;
  c.pin_d4 = Y6_GPIO_NUM;
  c.pin_d5 = Y7_GPIO_NUM;
  c.pin_d6 = Y8_GPIO_NUM;
  c.pin_d7 = Y9_GPIO_NUM;
  c.pin_xclk = XCLK_GPIO_NUM;
  c.pin_pclk = PCLK_GPIO_NUM;
  c.pin_vsync = VSYNC_GPIO_NUM;
  c.pin_href = HREF_GPIO_NUM;
  c.pin_sccb_sda = SIOD_GPIO_NUM;
  c.pin_sccb_scl = SIOC_GPIO_NUM;
  c.pin_pwdn = PWDN_GPIO_NUM;
  c.pin_reset = RESET_GPIO_NUM;
  c.xclk_freq_hz = 20000000;
  c.pixel_format = PIXFORMAT_JPEG;

  if (psramFound()) {
    c.frame_size = FRAMESIZE_SVGA;
    c.jpeg_quality = 12;
    c.fb_count = 2;
    c.fb_location = CAMERA_FB_IN_PSRAM;
    c.grab_mode = CAMERA_GRAB_LATEST;
  } else {
    c.frame_size = FRAMESIZE_QVGA;
    c.jpeg_quality = 15;
    c.fb_count = 1;
    c.fb_location = CAMERA_FB_IN_DRAM;
    c.grab_mode = CAMERA_GRAB_WHEN_EMPTY;
  }

  esp_err_t err = esp_camera_init(&c);
  if (err != ESP_OK) {
    Serial.printf("camera init failed: 0x%x\n", err);
    return false;
  }
  return true;
}

static bool authorized(httpd_req_t *req) {
  if (cfgToken.isEmpty()) return false;

  char header[96];
  if (httpd_req_get_hdr_value_str(req, "Authorization", header, sizeof(header)) == ESP_OK) {
    if (String(header) == "Bearer " + cfgToken) return true;
  }
  char query[160];
  if (httpd_req_get_url_query_str(req, query, sizeof(query)) == ESP_OK) {
    char value[96];
    if (httpd_query_key_value(query, "token", value, sizeof(value)) == ESP_OK &&
        cfgToken == value) {
      return true;
    }
  }
  return false;
}

static esp_err_t deny(httpd_req_t *req) {
  httpd_resp_set_status(req, "401 Unauthorized");
  return httpd_resp_send(req, "unauthorized", HTTPD_RESP_USE_STRLEN);
}

static esp_err_t healthHandler(httpd_req_t *req) {
  return httpd_resp_send(req, "ok", HTTPD_RESP_USE_STRLEN);
}

static esp_err_t unavailable(httpd_req_t *req) {
  httpd_resp_set_status(req, "503 Service Unavailable");
  return httpd_resp_send(req, "standby", HTTPD_RESP_USE_STRLEN);
}

static esp_err_t captureHandler(httpd_req_t *req) {
  if (!authorized(req)) return deny(req);
  if (standby) return unavailable(req);

  camera_fb_t *fb = esp_camera_fb_get();
  if (!fb) {
    httpd_resp_send_500(req);
    return ESP_FAIL;
  }
  httpd_resp_set_type(req, "image/jpeg");
  httpd_resp_set_hdr(req, "Cache-Control", "no-store");
  esp_err_t res = httpd_resp_send(req, (const char *)fb->buf, fb->len);
  esp_camera_fb_return(fb);
  return res;
}

static esp_err_t statusHandler(httpd_req_t *req) {
  if (!authorized(req)) return deny(req);

  sensor_t *s = standby ? nullptr : esp_camera_sensor_get();
  char body[512];
  snprintf(body, sizeof(body),
           "{\"fw\":\"%s\",\"uptime_s\":%lu,\"rssi\":%d,\"ip\":\"%s\",\"mac\":\"%s\","
           "\"free_heap\":%u,\"free_psram\":%u,\"framesize\":%d,\"quality\":%d,"
           "\"vflip\":%d,\"hmirror\":%d,\"standby\":%d,\"reset_reason\":%d}",
           FW_VERSION, millis() / 1000, WiFi.RSSI(), WiFi.localIP().toString().c_str(),
           WiFi.macAddress().c_str(), ESP.getFreeHeap(), ESP.getFreePsram(),
           s ? s->status.framesize : -1, s ? s->status.quality : -1, s ? s->status.vflip : 0,
           s ? s->status.hmirror : 0, standby ? 1 : 0, (int)esp_reset_reason());
  httpd_resp_set_type(req, "application/json");
  return httpd_resp_send(req, body, HTTPD_RESP_USE_STRLEN);
}

static esp_err_t controlHandler(httpd_req_t *req) {
  if (!authorized(req)) return deny(req);

  char query[96], var[24], val[16];
  if (httpd_req_get_url_query_str(req, query, sizeof(query)) != ESP_OK ||
      httpd_query_key_value(query, "var", var, sizeof(var)) != ESP_OK ||
      httpd_query_key_value(query, "val", val, sizeof(val)) != ESP_OK) {
    httpd_resp_send_err(req, HTTPD_400_BAD_REQUEST, "var and val required");
    return ESP_FAIL;
  }

  int v = atoi(val);
  if (!strcmp(var, "standby")) {
    if (v && !standby) {
      standby = true;
      delay(300);
      esp_camera_deinit();
    } else if (!v && standby) {
      if (!initCamera()) {
        httpd_resp_send_500(req);
        return ESP_FAIL;
      }
      standby = false;
    }
    return httpd_resp_send(req, "ok", HTTPD_RESP_USE_STRLEN);
  }
  if (standby) return unavailable(req);

  sensor_t *s = esp_camera_sensor_get();
  int res = -1;
  if (!strcmp(var, "framesize") && v >= FRAMESIZE_QVGA && v <= FRAMESIZE_UXGA)
    res = s->set_framesize(s, (framesize_t)v);
  else if (!strcmp(var, "quality") && v >= 4 && v <= 63) res = s->set_quality(s, v);
  else if (!strcmp(var, "vflip")) res = s->set_vflip(s, v != 0);
  else if (!strcmp(var, "hmirror")) res = s->set_hmirror(s, v != 0);
  else if (!strcmp(var, "brightness")) res = s->set_brightness(s, constrain(v, -2, 2));
  else if (!strcmp(var, "contrast")) res = s->set_contrast(s, constrain(v, -2, 2));

  if (res != 0) {
    httpd_resp_send_err(req, HTTPD_400_BAD_REQUEST, "unknown or invalid setting");
    return ESP_FAIL;
  }
  return httpd_resp_send(req, "ok", HTTPD_RESP_USE_STRLEN);
}

static const char *STREAM_CT = "multipart/x-mixed-replace;boundary=frame";
static const char *STREAM_BOUNDARY = "\r\n--frame\r\n";

static esp_err_t streamHandler(httpd_req_t *req) {
  if (!authorized(req)) return deny(req);
  if (standby) return unavailable(req);

  httpd_resp_set_type(req, STREAM_CT);
  httpd_resp_set_hdr(req, "Cache-Control", "no-store");

  esp_err_t res = ESP_OK;
  while (res == ESP_OK) {
    if (standby) break;
    camera_fb_t *fb = esp_camera_fb_get();
    if (!fb) {
      res = ESP_FAIL;
      break;
    }
    char part[80];
    size_t n = snprintf(part, sizeof(part),
                        "Content-Type: image/jpeg\r\nContent-Length: %u\r\n\r\n",
                        (unsigned)fb->len);
    res = httpd_resp_send_chunk(req, STREAM_BOUNDARY, strlen(STREAM_BOUNDARY));
    if (res == ESP_OK) res = httpd_resp_send_chunk(req, part, n);
    if (res == ESP_OK) res = httpd_resp_send_chunk(req, (const char *)fb->buf, fb->len);
    esp_camera_fb_return(fb);
  }
  return res;
}

static void startServers() {
  httpd_config_t cfg = HTTPD_DEFAULT_CONFIG();
  cfg.server_port = 80;
  cfg.max_uri_handlers = 8;

  httpd_uri_t health = {"/health", HTTP_GET, healthHandler, nullptr};
  httpd_uri_t capture = {"/capture", HTTP_GET, captureHandler, nullptr};
  httpd_uri_t status = {"/status", HTTP_GET, statusHandler, nullptr};
  httpd_uri_t control = {"/control", HTTP_GET, controlHandler, nullptr};

  if (httpd_start(&apiServer, &cfg) == ESP_OK) {
    httpd_register_uri_handler(apiServer, &health);
    httpd_register_uri_handler(apiServer, &capture);
    httpd_register_uri_handler(apiServer, &status);
    httpd_register_uri_handler(apiServer, &control);
  }

  cfg.server_port = 81;
  cfg.ctrl_port += 1;
  httpd_uri_t stream = {"/stream", HTTP_GET, streamHandler, nullptr};
  if (httpd_start(&streamServer, &cfg) == ESP_OK) {
    httpd_register_uri_handler(streamServer, &stream);
  }
}

static void connectWifi() {
  WiFi.mode(WIFI_STA);
  WiFi.setHostname("habitacion-cam");
  if (cfgSsid.isEmpty()) {
    Serial.println("wifi not configured: use 'ssid', 'pass' and 'reboot'");
    return;
  }
  WiFi.setSleep(false);
  WiFi.setAutoReconnect(true);
  WiFi.begin(cfgSsid.c_str(), cfgPass.c_str());
}

static void scanNetworks() {
  WiFi.mode(WIFI_STA);
  int n = WiFi.scanNetworks();
  Serial.printf("%d networks\n", n);
  for (int i = 0; i < n; i++) {
    Serial.printf("  %-32s ch%-2d %4d dBm  %s\n", WiFi.SSID(i).c_str(), WiFi.channel(i),
                  WiFi.RSSI(i), WiFi.encryptionType(i) == WIFI_AUTH_OPEN ? "open" : "secured");
  }
  WiFi.scanDelete();
}

static void printStatus() {
  Serial.printf("fw %s | wifi %s | ip %s | rssi %d | ssid '%s' | token %s | psram %u | reset %d\n",
                FW_VERSION, WiFi.status() == WL_CONNECTED ? "up" : "down",
                WiFi.localIP().toString().c_str(), WiFi.RSSI(), cfgSsid.c_str(),
                cfgToken.isEmpty() ? "unset" : "set", ESP.getPsramSize(),
                (int)esp_reset_reason());
}

static void handleSerialLine(String line) {
  line.trim();
  if (line.isEmpty()) return;

  int sp = line.indexOf(' ');
  String cmd = sp < 0 ? line : line.substring(0, sp);
  String arg = sp < 0 ? "" : line.substring(sp + 1);

  if (cmd == "scan") scanNetworks();
  else if (cmd == "ssid") { saveKey("ssid", arg); Serial.println("ssid saved"); }
  else if (cmd == "pass") { saveKey("pass", arg); Serial.println("pass saved"); }
  else if (cmd == "token") { saveKey("token", arg); Serial.println("token saved"); }
  else if (cmd == "status") printStatus();
  else if (cmd == "reboot") ESP.restart();
  else if (cmd == "wipe") {
    prefs.begin("cam", false);
    prefs.clear();
    prefs.end();
    Serial.println("config wiped");
  } else Serial.println("commands: scan ssid pass token status reboot wipe");
}

static void pollSerial() {
  static String buf;
  while (Serial.available()) {
    char ch = Serial.read();
    if (ch == '\n' || ch == '\r') {
      handleSerialLine(buf);
      buf = "";
    } else if (buf.length() < 160) {
      buf += ch;
    }
  }
}

void setup() {
  Serial.begin(115200);
  delay(300);
  pinMode(FLASH_LED_GPIO, OUTPUT);
  digitalWrite(FLASH_LED_GPIO, LOW);

  loadConfig();
  Serial.printf("\nhabitacion-cam fw %s\n", FW_VERSION);

  if (!initCamera()) {
    delay(2000);
    ESP.restart();
  }

  connectWifi();
  startServers();

  if (!cfgToken.isEmpty()) {
    ArduinoOTA.setHostname("habitacion-cam");
    ArduinoOTA.setPassword(cfgToken.c_str());
    ArduinoOTA.begin();
  }
  lastWifiOk = millis();
  printStatus();
}

void loop() {
  pollSerial();
  ArduinoOTA.handle();

  if (WiFi.status() == WL_CONNECTED) {
    lastWifiOk = millis();
  } else if (!cfgSsid.isEmpty() && millis() - lastWifiOk > WIFI_RESTART_AFTER_MS) {
    ESP.restart();
  }
  delay(20);
}
