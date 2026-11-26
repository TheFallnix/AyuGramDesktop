// This is the source code of AyuGram for Desktop.
//
// We do not and cannot prevent the use of our code,
// but be respectful and credit the original author.
//
// Copyright @Radolyn, 2026
#include "ayu/features/filters/filters_controller.h"

#include "ayu/ayu_settings.h"
#include "ayu/features/filters/filters_cache_controller.h"
#include "ayu/features/filters/filters_utils.h"
#include "ayu/ffi/ayu_rust_bridge.h"
#include "ayu/utils/telegram_helpers.h"
#include "data/data_peer.h"
#include "data/data_peer_id.h"
#include "data/data_session.h"
#include "data/data_user.h"
#include "history/history.h"
#include "history/history_item.h"
#include "history/history_item_components.h"

#include <unordered_set>

namespace FiltersController {
namespace {

std::unordered_set<long long> showingFilteredMessages;

bool filterBlocked(const not_null<HistoryItem*> item) {
	if (item->from() != item->history()->peer) {
		if (isBlocked(item)) {
			return true;
		}
	}
	if (const auto bot = item->viaBot()) {
		if (isBlocked(bot)) {
			return true;
		}
	}
	return false;
}

} // namespace

bool isEnabled(not_null<PeerData*> peer) {
	const auto &settings = AyuSettings::getInstance();
	return settings.filtersEnabled()
		&& (settings.filtersEnabledInChats() || peer->isBroadcast());
}

bool isBlocked(const not_null<HistoryItem*> item) {
	const auto &settings = AyuSettings::getInstance();

	auto shadowBanMatched = false;
	const auto blocked = [&]() -> bool {
		const auto isShadowBanned = [&](PeerData *peer) {
			return peer
				&& (peer->isUser() || peer->isBroadcast())
				&& settings.isShadowBanned(getDialogIdFromPeer(peer));
		};

		if (isShadowBanned(item->from())
			&& item->from()->id != item->history()->peer->id) {
			shadowBanMatched = true;
			return true;
		}

		if (item->from()->isUser()
			&& item->from()->asUser()->isBlocked()) {
			return item->from()->asUser()->id != item->history()->peer->id;
		}

		if (const auto forwarded = item->Get<HistoryMessageForwarded>()) {
			if (const auto originalSender = forwarded->originalSender) {
				const auto originalShadowBanned = isShadowBanned(originalSender);
				if (originalShadowBanned
					|| (originalSender->isUser()
						&& originalSender->asUser()->isBlocked())) {
					shadowBanMatched = originalShadowBanned;
					return true;
				}
			}
		}
		return false;
	}();

	return settings.filtersEnabled()
		&& (shadowBanMatched || settings.hideFromBlocked())
		&& blocked;
}

bool isBlocked(const not_null<PeerData*> peer) {
	const auto &settings = AyuSettings::getInstance();
	return settings.filtersEnabled() && (
		(peer->isUser()
			&& peer->asUser()->isBlocked()
			&& settings.hideFromBlocked())
		|| ((peer->isUser() || peer->isBroadcast())
			&& settings.isShadowBanned(getDialogIdFromPeer(peer))));
}

bool filtered(const not_null<HistoryItem*> item) {
	if (showingFilteredMessages.contains(item->history()->peer->id.value)) {
		return false;
	}

	const auto &settings = AyuSettings::getInstance();
	if (!settings.filtersEnabled()) {
		return false;
	}

	if (item->out()) {
		return false;
	}

	if (filterBlocked(item)) {
		FiltersCacheController::putHiddenBlockedMessage(item);
		return true;
	}

	if (!isEnabled(item->history()->peer)) {
		return false;
	}

	const auto bridge = Ayu::Ffi::RustBridge::Global();
	if (!bridge) {
		return false;
	}
	const auto group = item->history()->owner().groups().find(item);
	const auto text = FilterUtils::extractAllText(item, group);
	if (text.isEmpty()) {
		return false;
	}
	return bridge->evaluateMessageFilter(
		std::int64_t(getDialogIdFromPeer(item->history()->peer)),
		std::int32_t(item->id.bare),
		std::int64_t(getDialogIdFromPeer(item->from())),
		item->out(),
		text);
}

std::optional<bool> filteredMessagesShown(not_null<PeerData*> peer) {
	if (!showingFilteredMessages.contains(peer->id.value)
		&& !FiltersCacheController::hasFilteredMessages(peer)) {
		return std::nullopt;
	}
	return showingFilteredMessages.contains(peer->id.value);
}

void toggleFilteredMessagesShown(not_null<PeerData*> peer) {
	if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
		bridge->toggleFilteredMessagesShown(
			std::int64_t(getDialogIdFromPeer(peer)));
	}
	if (showingFilteredMessages.contains(peer->id.value)) {
		showingFilteredMessages.erase(peer->id.value);
	} else {
		showingFilteredMessages.insert(peer->id.value);
	}
	FiltersCacheController::fireUpdate();
}

void invalidate(not_null<HistoryItem*> item) {
	const auto &settings = AyuSettings::getInstance();
	if (!settings.filtersEnabled()) {
		return;
	}
	FiltersCacheController::invalidate(item);
}

} // namespace FiltersController
