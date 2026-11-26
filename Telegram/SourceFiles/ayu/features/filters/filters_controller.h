// This is the source code of AyuGram for Desktop.
//
// We do not and cannot prevent the use of our code,
// but be respectful and credit the original author.
//
// Copyright @Radolyn, 2026
#pragma once

#include <optional>

namespace FiltersController {

bool isEnabled(not_null<PeerData*> peer);
bool isBlocked(not_null<HistoryItem*> item);
bool isBlocked(not_null<PeerData*> peer);
bool filtered(not_null<HistoryItem*> historyItem);
std::optional<bool> filteredMessagesShown(not_null<PeerData*> peer);
void toggleFilteredMessagesShown(not_null<PeerData*> peer);

void invalidate(not_null<HistoryItem*> item);

} // namespace FiltersController
