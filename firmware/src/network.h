#pragma once

#include <Arduino.h>

void networkBegin();
void networkLoop();
bool networkPortalActive();
void networkStartPortal();
String networkScanJson();
