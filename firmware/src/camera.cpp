#include "camera.h"

#include <Arduino.h>
#include <esp_camera.h>

#include "pins.h"

static bool running = false;
static uint8_t failedFrames = 0;
static volatile int framesInUse = 0;
static const uint32_t STOP_WAIT_MS = 3000;
static const uint8_t STUCK_AFTER_FAILURES = 30;

struct Control {
  const char *name;
  int low;
  int high;
  int (*apply)(sensor_t *, int);
};

static const Control CONTROLS[] = {
    {"framesize", FRAMESIZE_QVGA, FRAMESIZE_UXGA, [](sensor_t *s, int v) { return s->set_framesize(s, (framesize_t)v); }},
    {"quality", 4, 63, [](sensor_t *s, int v) { return s->set_quality(s, v); }},
    {"brightness", -2, 2, [](sensor_t *s, int v) { return s->set_brightness(s, v); }},
    {"contrast", -2, 2, [](sensor_t *s, int v) { return s->set_contrast(s, v); }},
    {"saturation", -2, 2, [](sensor_t *s, int v) { return s->set_saturation(s, v); }},
    {"sharpness", -2, 2, [](sensor_t *s, int v) { return s->set_sharpness(s, v); }},
    {"vflip", 0, 1, [](sensor_t *s, int v) { return s->set_vflip(s, v); }},
    {"hmirror", 0, 1, [](sensor_t *s, int v) { return s->set_hmirror(s, v); }},
    {"awb", 0, 1, [](sensor_t *s, int v) { return s->set_whitebal(s, v); }},
    {"wb_mode", 0, 4, [](sensor_t *s, int v) { return s->set_wb_mode(s, v); }},
    {"aec", 0, 1, [](sensor_t *s, int v) { return s->set_exposure_ctrl(s, v); }},
    {"aec2", 0, 1, [](sensor_t *s, int v) { return s->set_aec2(s, v); }},
    {"ae_level", -2, 2, [](sensor_t *s, int v) { return s->set_ae_level(s, v); }},
    {"agc", 0, 1, [](sensor_t *s, int v) { return s->set_gain_ctrl(s, v); }},
    {"gainceiling", 0, 6, [](sensor_t *s, int v) { return s->set_gainceiling(s, (gainceiling_t)v); }},
    {"lenc", 0, 1, [](sensor_t *s, int v) { return s->set_lenc(s, v); }},
};

static camera_config_t buildConfig() {
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
    c.frame_size = FRAMESIZE_UXGA;
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
  return c;
}

bool cameraStart() {
  if (running) return true;
  camera_config_t config = buildConfig();
  esp_err_t err = esp_camera_init(&config);
  if (err != ESP_OK) {
    Serial.printf("camera init failed: 0x%x\n", err);
    return false;
  }
  running = true;
  failedFrames = 0;
  sensor_t *sensor = esp_camera_sensor_get();
  if (sensor != nullptr && psramFound()) sensor->set_framesize(sensor, FRAMESIZE_SVGA);
  return true;
}

camera_fb_t *cameraFrame() {
  camera_fb_t *frame = running ? esp_camera_fb_get() : nullptr;
  if (frame != nullptr) framesInUse = framesInUse + 1;
  return frame;
}

void cameraRelease(camera_fb_t *frame) {
  esp_camera_fb_return(frame);
  framesInUse = framesInUse - 1;
}

void cameraStop() {
  if (!running) return;
  running = false;
  uint32_t waited = 0;
  while (framesInUse > 0 && waited < STOP_WAIT_MS) {
    delay(20);
    waited += 20;
  }
  esp_camera_deinit();
}

bool cameraRunning() { return running; }

bool cameraControl(const char *name, int value) {
  sensor_t *sensor = running ? esp_camera_sensor_get() : nullptr;
  if (sensor == nullptr) return false;
  for (const Control &control : CONTROLS) {
    if (strcmp(control.name, name) == 0) {
      return value >= control.low && value <= control.high && control.apply(sensor, value) == 0;
    }
  }
  return false;
}

void cameraDescribe(int &framesize, int &quality, int &vflip, int &hmirror) {
  sensor_t *sensor = running ? esp_camera_sensor_get() : nullptr;
  framesize = sensor ? sensor->status.framesize : -1;
  quality = sensor ? sensor->status.quality : -1;
  vflip = sensor ? sensor->status.vflip : 0;
  hmirror = sensor ? sensor->status.hmirror : 0;
}

void cameraNoteFrame(bool ok) {
  failedFrames = ok ? 0 : min<int>(failedFrames + 1, 255);
}

bool cameraStuck() { return running && failedFrames >= STUCK_AFTER_FAILURES; }
