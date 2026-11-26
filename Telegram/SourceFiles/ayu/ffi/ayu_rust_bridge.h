#pragma once

#include <QtCore/QByteArray>
#include <QtCore/QString>

#include <cstddef>
#include <cstdint>
#include <memory>
#include <optional>
#include <vector>

namespace Ayu::Ffi {

struct BridgeMessageRow {
	std::int64_t dialogId = 0;
	std::int32_t msgId = 0;
	std::uint64_t senderId = 0;
	std::int32_t replyToMsgId = 0;
	std::int32_t dateUnix = 0;
	std::int32_t editDateUnix = 0;
	std::uint32_t views = 0;
	std::uint32_t flags = 0;
	std::uint64_t groupedId = 0;
	QString authorName;
	QString text;
	std::uint8_t mediaKind = 0;
	QString mediaPath;
};

struct BridgeDialogRow {
	std::int64_t dialogId = 0;
	std::uint64_t peerId = 0;
	std::int32_t folderId = 0;
	QString title;
	QString draftText;
	std::int32_t topMessageId = 0;
	std::int32_t readInboxMaxId = 0;
	std::int32_t readOutboxMaxId = 0;
	std::int32_t lastMessageDate = 0;
	std::uint32_t unreadCount = 0;
	std::uint32_t unreadMentionsCount = 0;
	std::uint16_t flags = 0;
};

struct BridgeGhostConfig {
	bool ghostModeActive = false;
	bool sendReadMessages = true;
	bool sendReadStories = true;
	bool sendOnlinePackets = true;
	bool sendUploadProgress = true;
	bool sendOfflineAfterOnline = false;
	bool markReadAfterAction = true;
	bool useScheduledMessages = false;
	std::uint8_t sendWithoutSound = 0;
};

struct BridgeMemoryStats {
	std::size_t activeAccounts = 0;
	std::size_t residentWindows = 0;
	std::size_t cachedWindowMessages = 0;
	std::size_t cachedWindowBytes = 0;
	std::size_t residentDialogWindows = 0;
	std::size_t cachedDialogRows = 0;
	std::size_t mediaCacheEntries = 0;
	std::size_t mediaCacheBytes = 0;
	std::size_t mediaCacheLimitBytes = 0;
	std::size_t processRssBytes = 0;
};

struct BridgeAccountSnapshot {
	std::uint64_t accountId = 0;
	std::uint64_t userId = 0;
	std::uint64_t sessionUniqueId = 0;
	std::int32_t mainDcId = 0;
	std::uint8_t authState = 0;
	std::uint8_t lifecycleState = 0;
	bool isTestDc = false;
	bool isPremium = false;
	std::uint32_t unreadCount = 0;
	bool unreadMuted = true;
};

struct BridgeUnreadBadge {
	std::uint32_t totalUnread = 0;
	bool allMuted = true;
};

struct BridgeSyncState {
	std::int32_t pts = 0;
	std::int32_t qts = 0;
	std::int32_t date = 0;
	std::int32_t seq = 0;
};

struct BridgePtsOutcome {
	bool applied = false;
	bool gapDetected = false;
	bool duplicate = false;
	std::int32_t currentPts = 0;
};

struct BridgeTransferProgress {
	std::uint64_t taskId = 0;
	std::uint64_t fileId = 0;
	std::uint64_t transferredBytes = 0;
	std::uint64_t totalBytes = 0;
	std::size_t partSize = 0;
	std::uint32_t partsTotal = 0;
	std::uint32_t partsCompleted = 0;
	std::uint8_t state = 0;
	bool isBigFile = false;
};

struct BridgeRegexFilter {
	QByteArray id;
	QString pattern;
	bool enabled = false;
	bool reversed = false;
	bool caseInsensitive = false;
	bool hasDialogId = false;
	std::int64_t dialogId = 0;
};

struct BridgeRegexExclusion {
	std::int64_t dialogId = 0;
	QByteArray filterId;
};

struct BridgeRawRecord {
	std::int64_t fakeId = 0;
	QByteArray payload;
};

class RustBridge final {
public:
	static std::unique_ptr<RustBridge> Create(
		const QString &dbPath,
		std::size_t mediaCacheLimitMb);
	static void InitializeGlobal(
		const QString &dbPath,
		std::size_t mediaCacheLimitMb);
	[[nodiscard]] static RustBridge *Global();

	~RustBridge();

	RustBridge(const RustBridge &) = delete;
	RustBridge &operator=(const RustBridge &) = delete;

	[[nodiscard]] bool isReady() const;
	[[nodiscard]] QString lastError() const;

	bool registerAccount(
		std::uint64_t accountId,
		std::uint64_t userId,
		bool isTestDc);
	bool activateAccount(std::uint64_t accountId);
	bool removeAccount(std::uint64_t accountId);

	[[nodiscard]] std::optional<BridgeAccountSnapshot> importMtpAuthorization(
		std::uint64_t accountId,
		const QByteArray &serialized);
	[[nodiscard]] QByteArray exportMtpAuthorization(std::uint64_t accountId);
	bool completeAuthorization(
		std::uint64_t accountId,
		std::uint64_t userId,
		std::int32_t mainDcId,
		bool isPremium);
	bool beginLogout(std::uint64_t accountId);
	bool finishLogout(std::uint64_t accountId);
	bool forceLogout(std::uint64_t accountId);
	bool pushLoginCode(std::uint64_t accountId, const QString &code);
	[[nodiscard]] QString takeLoginCode(std::uint64_t accountId);
	bool setAccountUnreadState(
		std::uint64_t accountId,
		std::uint32_t unreadCount,
		bool muted);
	bool setAccountSeparateWindow(
		std::uint64_t accountId,
		bool hasWindow);
	bool markAccountOnline(std::uint64_t accountId);
	[[nodiscard]] std::vector<std::uint64_t> collectAccountsForOfflinePacket();
	[[nodiscard]] BridgeUnreadBadge computeUnreadBadge() const;
	[[nodiscard]] std::vector<std::uint64_t> orderedAccountIds(
		const std::vector<std::uint64_t> &orderUniqueIds) const;
	[[nodiscard]] std::vector<std::uint64_t> redundantAccountIds() const;
	[[nodiscard]] std::size_t maxAccountsLimit() const;
	[[nodiscard]] std::size_t authedAccountsCount() const;
	[[nodiscard]] std::optional<BridgeAccountSnapshot> accountSnapshot(
		std::uint64_t accountId) const;

	bool setGhostConfig(
		std::uint64_t accountId,
		const BridgeGhostConfig &config);
	[[nodiscard]] std::optional<BridgeGhostConfig> ghostConfig(
		std::uint64_t accountId);

	[[nodiscard]] bool shouldSendReadAck(std::uint64_t accountId) const;
	[[nodiscard]] bool shouldSendOnlinePacket(std::uint64_t accountId) const;

	bool upsertMessage(
		std::uint64_t accountId,
		const BridgeMessageRow &row);
	std::size_t upsertMessageBatch(
		std::uint64_t accountId,
		const std::vector<BridgeMessageRow> &rows);
	[[nodiscard]] std::vector<BridgeMessageRow> loadViewportMessages(
		std::uint64_t accountId,
		std::int64_t dialogId,
		std::int32_t centerMsgId,
		std::size_t limit);
	[[nodiscard]] std::vector<BridgeMessageRow> scrollViewportOlder(
		std::uint64_t accountId,
		std::int64_t dialogId,
		std::size_t batchSize);
	[[nodiscard]] std::vector<BridgeMessageRow> scrollViewportNewer(
		std::uint64_t accountId,
		std::int64_t dialogId,
		std::size_t batchSize);
	std::size_t deleteMessages(
		std::uint64_t accountId,
		std::int64_t dialogId,
		const std::vector<std::int32_t> &msgIds);
	std::size_t clearDialogHistoryUpTo(
		std::uint64_t accountId,
		std::int64_t dialogId,
		std::int32_t maxMsgId);
	void unloadDialogViewport(
		std::uint64_t accountId,
		std::int64_t dialogId);
	[[nodiscard]] std::vector<BridgeMessageRow> searchDialogMessages(
		std::uint64_t accountId,
		std::int64_t dialogId,
		const QString &query,
		std::size_t limit);

	bool upsertDialog(
		std::uint64_t accountId,
		const BridgeDialogRow &row);
	std::size_t upsertDialogBatch(
		std::uint64_t accountId,
		const std::vector<BridgeDialogRow> &rows);
	bool updateDialogReadState(
		std::uint64_t accountId,
		std::int64_t dialogId,
		std::int32_t readInboxMaxId,
		std::uint32_t unreadCount);
	bool removeDialog(
		std::uint64_t accountId,
		std::int64_t dialogId);
	[[nodiscard]] std::vector<BridgeDialogRow> loadDialogSlice(
		std::uint64_t accountId,
		std::int32_t folderId,
		std::size_t offset,
		std::size_t limit);
	[[nodiscard]] std::vector<BridgeDialogRow> searchDialogs(
		std::uint64_t accountId,
		const QString &query,
		std::size_t limit);

	bool recordDeletedMessage(
		std::uint64_t accountId,
		const BridgeMessageRow &row);
	[[nodiscard]] std::vector<BridgeMessageRow> loadDeletedMessages(
		std::uint64_t accountId,
		std::int64_t dialogId,
		std::size_t limit);
	[[nodiscard]] bool hasDeletedMessages(
		std::uint64_t accountId,
		std::int64_t dialogId) const;
	bool recordEditedMessage(
		std::uint64_t accountId,
		const BridgeMessageRow &row);
	[[nodiscard]] std::vector<BridgeMessageRow> loadEditedMessages(
		std::uint64_t accountId,
		std::int64_t dialogId,
		std::int32_t msgId,
		std::size_t limit);
	[[nodiscard]] bool hasEditedRevisions(
		std::uint64_t accountId,
		std::int64_t dialogId,
		std::int32_t msgId) const;

	bool putMediaCache(std::uint64_t key, const QByteArray &data);
	bool putScopedMediaCache(
		std::uint64_t accountId,
		std::uint8_t kind,
		std::uint64_t resourceId,
		const QByteArray &data);
	[[nodiscard]] QByteArray getScopedMediaCache(
		std::uint64_t accountId,
		std::uint8_t kind,
		std::uint64_t resourceId) const;
	std::size_t evictAccountMediaCache(
		std::uint64_t accountId,
		bool heavyOnly);
	std::size_t evictInactiveCaches();
	[[nodiscard]] BridgeMemoryStats memoryStats() const;

	bool initSyncState(
		std::uint64_t accountId,
		std::int32_t pts,
		std::int32_t qts,
		std::int32_t date,
		std::int32_t seq);
	[[nodiscard]] std::optional<BridgeSyncState> syncState(
		std::uint64_t accountId) const;
	std::optional<BridgePtsOutcome> applyPtsUpdate(
		std::uint64_t accountId,
		std::uint64_t channelId,
		std::int32_t pts,
		std::int32_t ptsCount);
	std::int32_t queueOutgoingMessage(
		std::uint64_t accountId,
		std::uint64_t randomId,
		const BridgeMessageRow &row);
	bool confirmOutgoingMessage(
		std::uint64_t accountId,
		std::uint64_t randomId,
		std::int32_t serverMsgId,
		std::int32_t dateUnix,
		std::int32_t pts,
		std::int32_t ptsCount);
	std::optional<BridgePtsOutcome> receiveIncomingMessage(
		std::uint64_t accountId,
		std::uint64_t channelId,
		std::int32_t pts,
		std::int32_t ptsCount,
		const BridgeMessageRow &row);
	std::uint64_t enqueueFileDownload(
		std::uint64_t accountId,
		std::uint16_t dcId,
		std::uint64_t fileId,
		std::uint64_t totalBytes,
		std::int32_t priority);
	std::uint64_t enqueueFileUpload(
		std::uint64_t accountId,
		std::uint16_t dcId,
		std::uint64_t fileId,
		std::uint64_t totalBytes);
	std::optional<BridgeTransferProgress> advanceFileTransfer(
		std::uint64_t taskId,
		std::size_t chunkBytes);
	bool cancelFileTransfer(std::uint64_t taskId);
	[[nodiscard]] std::size_t activeTransferInflightBytes() const;

	[[nodiscard]] std::vector<BridgeMessageRow> queryDeletedMessages(
		std::uint64_t accountId,
		std::int64_t dialogId,
		std::int64_t topicId,
		std::int32_t minId,
		std::int32_t maxId,
		std::size_t limit,
		const QString &searchQuery);
	std::size_t removeDeletedMessage(
		std::uint64_t accountId,
		std::int64_t dialogId,
		std::int32_t msgId);
	std::size_t clearDeletedMessages(
		std::uint64_t accountId,
		std::int64_t dialogId,
		std::int64_t topicId);
	[[nodiscard]] std::vector<BridgeMessageRow> queryEditedMessages(
		std::uint64_t accountId,
		std::int64_t dialogId,
		std::int32_t msgId,
		std::int64_t minId,
		std::int64_t maxId,
		std::size_t limit);
	bool upsertRegexFilter(const BridgeRegexFilter &filter);
	bool deleteRegexFilter(const QByteArray &id);
	std::size_t clearRegexFilters();
	[[nodiscard]] std::vector<BridgeRegexFilter> listRegexFilters();
	bool addRegexExclusion(
		std::int64_t dialogId,
		const QByteArray &filterId);
	bool removeRegexExclusion(
		std::int64_t dialogId,
		const QByteArray &filterId);
	std::size_t deleteExclusionsByFilterId(const QByteArray &filterId);
	std::size_t clearRegexExclusions();
	std::size_t rebuildFiltersCache();
	void setFiltersEnabled(
		bool enabled,
		bool enabledInChats,
		bool hideFromBlocked);
	void setShadowBanned(std::int64_t peerDialogId, bool banned);
	[[nodiscard]] bool isShadowBanned(std::int64_t peerDialogId) const;
	[[nodiscard]] bool evaluateMessageFilter(
		std::int64_t dialogId,
		std::int32_t msgId,
		std::int64_t senderDialogId,
		bool isOutgoing,
		const QString &text);
	void invalidateFilteredMessage(
		std::int64_t dialogId,
		std::int32_t msgId);
	[[nodiscard]] bool hasFilteredMessages(std::int64_t dialogId) const;
	bool toggleFilteredMessagesShown(std::int64_t dialogId);
	void hideMessage(std::int64_t dialogId, std::int32_t msgId);
	[[nodiscard]] bool isMessageHidden(
		std::int64_t dialogId,
		std::int32_t msgId) const;
	void armDisableGhostOnStoryClose(std::uint64_t accountId);
	bool consumeDisableGhostOnStoryClose(std::uint64_t accountId);

	[[nodiscard]] std::vector<BridgeRegexExclusion> listRegexExclusions();
	[[nodiscard]] bool validateRegexPattern(
		const QString &pattern,
		QString &errorOut) const;
	std::int64_t insertDeletedRecord(
		std::int64_t userId,
		std::int64_t dialogId,
		std::int64_t topicId,
		std::int32_t msgId,
		const QString &text,
		const QByteArray &payload);
	[[nodiscard]] std::vector<BridgeRawRecord> queryDeletedRecords(
		std::int64_t userId,
		std::int64_t dialogId,
		std::int64_t topicId,
		std::int64_t minId,
		std::int64_t maxId,
		std::size_t limit,
		const QString &searchQuery);
	[[nodiscard]] bool hasDeletedRecords(
		std::int64_t userId,
		std::int64_t dialogId,
		std::int64_t topicId) const;
	std::size_t removeDeletedRecord(
		std::int64_t userId,
		std::int64_t dialogId,
		std::int32_t msgId);
	std::size_t clearDeletedRecords(
		std::int64_t userId,
		std::int64_t dialogId,
		std::int64_t topicId);
	std::int64_t insertEditedRecord(
		std::int64_t userId,
		std::int64_t dialogId,
		std::int32_t msgId,
		const QByteArray &payload);
	[[nodiscard]] std::vector<BridgeRawRecord> queryEditedRecords(
		std::int64_t userId,
		std::int64_t dialogId,
		std::int32_t msgId,
		std::int64_t minId,
		std::int64_t maxId,
		std::size_t limit);
	[[nodiscard]] bool hasEditedRecords(
		std::int64_t userId,
		std::int64_t dialogId,
		std::int32_t msgId) const;

private:
	struct Impl;
	explicit RustBridge(std::unique_ptr<Impl> impl);

	std::unique_ptr<Impl> _impl;

};

} // namespace Ayu::Ffi
