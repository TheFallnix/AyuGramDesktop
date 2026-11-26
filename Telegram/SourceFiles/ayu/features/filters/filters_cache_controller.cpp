// This is the source code of AyuGram for Desktop.
//
// We do not and cannot prevent the use of our code,
// but be respectful and credit the original author.
//
// Copyright @Radolyn, 2026
#include "ayu/features/filters/filters_cache_controller.h"

#include "ayu/ayu_settings.h"
#include "ayu/ffi/ayu_rust_bridge.h"
#include "ayu/utils/telegram_helpers.h"
#include "data/data_peer.h"
#include "history/history.h"
#include "history/history_item.h"
#include "rpl/event_stream.h"

namespace FiltersCacheController {
namespace {

rpl::event_stream<> filtersUpdateStream;

} // namespace

void fireUpdate() {
	filtersUpdateStream.fire({});
}

rpl::producer<> updates() {
	return filtersUpdateStream.events();
}

void rebuildCache() {
	if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
		const auto &settings = AyuSettings::getInstance();
		bridge->setFiltersEnabled(
			settings.filtersEnabled(),
			settings.filtersEnabledInChats(),
			settings.hideFromBlocked());
		bridge->rebuildFiltersCache();
	}
}

bool hasFilteredMessages(not_null<PeerData*> peer) {
	const auto bridge = Ayu::Ffi::RustBridge::Global();
	return bridge
		? bridge->hasFilteredMessages(std::int64_t(getDialogIdFromPeer(peer)))
		: false;
}

void putHiddenBlockedMessage(not_null<HistoryItem*> item) {
	if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
		bridge->hideMessage(
			std::int64_t(getDialogIdFromPeer(item->history()->peer)),
			std::int32_t(item->id.bare));
	}
}

void invalidate(not_null<HistoryItem*> item) {
	if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
		bridge->invalidateFilteredMessage(
			std::int64_t(getDialogIdFromPeer(item->history()->peer)),
			std::int32_t(item->id.bare));
	}
}

} // namespace FiltersCacheController
