// This is the source code of AyuGram for Desktop.
//
// We do not and cannot prevent the use of our code,
// but be respectful and credit the original author.
//
// Copyright @Radolyn, 2026
#pragma once

#include "rpl/producer.h"

namespace FiltersCacheController {

void fireUpdate();
[[nodiscard]] rpl::producer<> updates();

void rebuildCache();

bool hasFilteredMessages(not_null<PeerData*> peer);
void putHiddenBlockedMessage(not_null<HistoryItem*> item);
void invalidate(not_null<HistoryItem*> item);

} // namespace FiltersCacheController
