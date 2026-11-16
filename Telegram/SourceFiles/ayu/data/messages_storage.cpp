// This is the source code of AyuGram for Desktop.
//
// We do not and cannot prevent the use of our code,
// but be respectful and credit the original author.
//
// Copyright @Radolyn, 2026
#include "ayu/data/messages_storage.h"

#include "ayu/data/ayu_database.h"
#include "ayu/ffi/ayu_rust_bridge.h"
#include "ayu/utils/ayu_mapper.h"
#include "ayu/utils/telegram_helpers.h"
#include "base/unixtime.h"
#include "core/application.h"
#include "data/data_forum_topic.h"
#include "data/data_session.h"
#include "history/history.h"
#include "history/history_item.h"
#include "history/history_item_components.h"
#include "main/main_account.h"
#include "main/main_domain.h"
#include "main/main_session.h"

namespace AyuMessages {
namespace {

[[nodiscard]] std::optional<std::uint64_t> AccountIdForAccount(
		const Main::Account &target) {
	for (const auto &[index, account] : Core::App().domain().accounts()) {
		if (account.get() == &target) {
			return std::uint64_t(index);
		}
	}
	return std::nullopt;
}

[[nodiscard]] std::optional<std::uint64_t> AccountIdForItem(
		not_null<HistoryItem*> item) {
	return AccountIdForAccount(item->history()->session().account());
}

[[nodiscard]] Ayu::Ffi::BridgeMessageRow ToBridgeMessageRow(
		const AyuMessageBase &msg) {
	return Ayu::Ffi::BridgeMessageRow{
		.dialogId = std::int64_t(msg.dialogId),
		.msgId = std::int32_t(msg.messageId),
		.senderId = std::uint64_t(msg.fromId),
		.replyToMsgId = std::int32_t(msg.replyMessageId),
		.dateUnix = std::int32_t(msg.date),
		.editDateUnix = std::int32_t(msg.editDate),
		.views = std::uint32_t(std::max(0, msg.views)),
		.flags = std::uint32_t(msg.flags),
		.groupedId = std::uint64_t(msg.groupedId),
		.text = QString::fromStdString(msg.text),
		.mediaKind = std::uint8_t(std::clamp(msg.documentType, 0, 255)),
		.mediaPath = QString::fromStdString(msg.mediaPath),
	};
}

} // namespace

template<typename DerivedMessage>
std::vector<AyuMessageBase> convertToBase(const std::vector<DerivedMessage> &messages) {
	std::vector<AyuMessageBase> based;
	based.reserve(messages.size());
	for (const auto &msg : messages) {
		based.push_back(static_cast<AyuMessageBase>(msg));
	}
	return based;
}

void map(not_null<HistoryItem*> item, AyuMessageBase &message) {
	const ID userId = item->history()->owner().session().userId().bare & PeerId::kChatTypeMask;

	message.userId = userId;
	message.dialogId = getDialogIdFromPeer(item->history()->peer);
	message.groupedId = item->groupId().raw();
	message.peerId = item->history()->peer->id.value & PeerId::kChatTypeMask;
	message.fromId = item->from()->id.value & PeerId::kChatTypeMask;
	if (item->topic()) {
		message.topicId = item->topicRootId().bare;
	} else {
		message.topicId = 0;
	}
	message.messageId = item->id.bare;
	message.date = item->date();
	message.flags = AyuMapper::mapItemFlagsToMTPFlags(item);

	if (const auto edited = item->Get<HistoryMessageEdited>()) {
		message.editDate = edited->date;
	} else {
		message.editDate = base::unixtime::now();
	}

	message.views = item->viewsCount();
	message.fwdFlags = 0;
	message.fwdFromId = 0;
	// message.fwdName
	message.fwdDate = 0;
	// message.fwdPostAuthor
	if (const auto msgsigned = item->Get<HistoryMessageSigned>()) {
		message.postAuthor = msgsigned->author.toStdString();
	}
	message.replyFlags = 0;
	message.replyMessageId = 0;
	message.replyPeerId = 0;
	message.replyTopId = 0;
	message.replyForumTopic = false;
	// message.replySerialized
	// message.replyMarkupSerialized
	message.entityCreateDate = base::unixtime::now();

	auto serializedText = AyuMapper::serializeTextWithEntities(item);
	message.text = serializedText.first;
	message.textEntities = serializedText.second;

	// todo: implement mapping
	message.mediaPath = "/";
	// message.hqThumbPath
	message.documentType = 0; // document type none
	// message.documentSerialized
	// message.thumbsSerialized
	// message.documentAttributesSerialized
	// message.mimeType
}

void addEditedMessage(not_null<HistoryItem *> item) {
	EditedMessage message;
	map(item, message);

	if (message.text.empty()) {
		return;
	}

	AyuDatabase::addEditedMessage(message);
	if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
		if (const auto accountId = AccountIdForItem(item)) {
			bridge->recordEditedMessage(
				*accountId,
				ToBridgeMessageRow(message));
		}
	}
}

std::vector<AyuMessageBase> getEditedMessages(not_null<HistoryItem*> item, ID minId, ID maxId, int totalLimit) {
	const ID userId = item->history()->owner().session().userId().bare & PeerId::kChatTypeMask;
	const auto dialogId = getDialogIdFromPeer(item->history()->peer);
	const auto msgId = item->id.bare;

	return convertToBase(AyuDatabase::getEditedMessages(userId, dialogId, msgId, minId, maxId, totalLimit));
}

bool hasRevisions(not_null<HistoryItem*> item) {
	const ID userId = item->history()->owner().session().userId().bare & PeerId::kChatTypeMask;
	const auto dialogId = getDialogIdFromPeer(item->history()->peer);
	const auto msgId = item->id.bare;
	if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
		if (const auto accountId = AccountIdForItem(item)) {
			if (bridge->hasEditedRevisions(
					*accountId,
					std::int64_t(dialogId),
					std::int32_t(msgId))) {
				return true;
			}
		}
	}

	return AyuDatabase::hasRevisions(userId, dialogId, msgId);
}

void addDeletedMessage(not_null<HistoryItem*> item) {
	DeletedMessage message;
	map(item, message);

	if (message.text.empty()) {
		return;
	}

	AyuDatabase::addDeletedMessage(message);
	if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
		if (const auto accountId = AccountIdForItem(item)) {
			bridge->recordDeletedMessage(
				*accountId,
				ToBridgeMessageRow(message));
		}
	}
}

std::vector<AyuMessageBase>
getDeletedMessages(not_null<PeerData*> peer, ID topicId, ID minId, ID maxId, int totalLimit, const QString &searchQuery) {
	const ID userId = peer->session().userId().bare & PeerId::kChatTypeMask;
	return convertToBase(
		AyuDatabase::getDeletedMessages(userId, getDialogIdFromPeer(peer), topicId, minId, maxId, totalLimit, searchQuery.toStdString()));
}

bool hasDeletedMessages(not_null<PeerData*> peer, ID topicId) {
	const ID userId = peer->session().userId().bare & PeerId::kChatTypeMask;
	const auto dialogId = getDialogIdFromPeer(peer);
	if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
		if (const auto accountId = AccountIdForAccount(peer->session().account())) {
			if (bridge->hasDeletedMessages(*accountId, std::int64_t(dialogId))) {
				return true;
			}
		}
	}
	return AyuDatabase::hasDeletedMessages(userId, dialogId, topicId);
}

void removeDeletedMessage(not_null<HistoryItem*> item) {
	const auto peer = item->history()->peer;
	const ID userId = peer->session().userId().bare & PeerId::kChatTypeMask;
	const auto dialogId = getDialogIdFromPeer(peer);
	AyuDatabase::removeDeletedMessage(userId, dialogId, item->id.bare);
	if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
		if (const auto accountId = AccountIdForItem(item)) {
			bridge->removeDeletedMessage(
				*accountId,
				std::int64_t(dialogId),
				std::int32_t(item->id.bare));
		}
	}
}

void clearDeletedMessages(not_null<PeerData*> peer, ID topicId) {
	const ID userId = peer->session().userId().bare & PeerId::kChatTypeMask;
	const auto dialogId = getDialogIdFromPeer(peer);
	AyuDatabase::clearDeletedMessages(userId, dialogId, topicId);
	if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
		if (const auto accountId = AccountIdForAccount(peer->session().account())) {
			bridge->clearDeletedMessages(
				*accountId,
				std::int64_t(dialogId),
				std::int64_t(topicId));
		}
	}
}

}
