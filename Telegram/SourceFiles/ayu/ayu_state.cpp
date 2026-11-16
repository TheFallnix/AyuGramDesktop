// This is the source code of AyuGram for Desktop.
//
// We do not and cannot prevent the use of our code,
// but be respectful and credit the original author.
//
// Copyright @Radolyn, 2026
#include "ayu/ayu_state.h"

#include "ayu/ayu_settings.h"
#include "ayu/ffi/ayu_rust_bridge.h"
#include "core/application.h"
#include "main/main_account.h"
#include "main/main_domain.h"
#include "main/main_session.h"

#include <optional>

namespace AyuState {
namespace {

[[nodiscard]] std::optional<std::uint64_t> AccountIdForSession(
		const Main::Session *session) {
	if (!session) {
		return std::nullopt;
	}
	for (const auto &[index, account] : Core::App().domain().accounts()) {
		if (account.get() == &session->account()) {
			return std::uint64_t(index);
		}
	}
	return std::nullopt;
}

} // namespace

std::unordered_map<PeerId, std::unordered_set<MsgId>> hiddenMessages;
Main::Session *disableGhostModeOnStoryCloseSession = nullptr;

void hide(PeerId peerId, MsgId messageId) {
	if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
		bridge->hideMessage(
			std::int64_t(peerId.value),
			std::int32_t(messageId.bare));
		return;
	}
	hiddenMessages[peerId].insert(messageId);
}

void hide(not_null<HistoryItem*> item) {
	hide(item->history()->peer->id, item->id);
}

bool isHidden(PeerId peerId, MsgId messageId) {
	if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
		return bridge->isMessageHidden(
			std::int64_t(peerId.value),
			std::int32_t(messageId.bare));
	}
	const auto it = hiddenMessages.find(peerId);
	if (it != hiddenMessages.end()) {
		return it->second.contains(messageId);
	}
	return false;
}

bool isHidden(not_null<HistoryItem*> item) {
	return isHidden(item->history()->peer->id, item->id);
}

void setDisableGhostModeOnStoryClose(Main::Session *session) {
	disableGhostModeOnStoryCloseSession = session;
	if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
		if (const auto accountId = AccountIdForSession(session)) {
			bridge->armDisableGhostOnStoryClose(*accountId);
		}
	}
}

void disableGhostModeOnStoryClose(Main::Session *session) {
	if (disableGhostModeOnStoryCloseSession != session) {
		return;
	}
	disableGhostModeOnStoryCloseSession = nullptr;
	if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
		if (const auto accountId = AccountIdForSession(session)) {
			bridge->consumeDisableGhostOnStoryClose(*accountId);
		}
	}
	if (session) {
		AyuSettings::ghost(session).setGhostModeEnabled(false);
	}
}

}
