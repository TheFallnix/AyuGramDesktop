#include "ayu/ffi/ayu_rust_bridge.h"

#include "ayu-ffi/src/lib.rs.h"
#include "rust/cxx.h"

#include <utility>

namespace Ayu::Ffi {
namespace {

std::unique_ptr<RustBridge> GlobalBridge;

[[nodiscard]] rust::String ToRustString(const QString &value) {
	const auto utf8 = value.toUtf8();
	return rust::String(utf8.constData(), std::size_t(utf8.size()));
}

[[nodiscard]] QString FromRustString(const rust::String &value) {
	return QString::fromUtf8(value.data(), int(value.size()));
}

[[nodiscard]] BridgeAccountSnapshot FromFfiSnapshot(
		const ayu::ffi::FfiAccountSnapshot &snap) {
	return BridgeAccountSnapshot{
		.accountId = snap.account_id,
		.userId = snap.user_id,
		.sessionUniqueId = snap.session_unique_id,
		.mainDcId = snap.main_dc_id,
		.authState = snap.auth_state,
		.lifecycleState = snap.lifecycle_state,
		.isTestDc = snap.is_test_dc,
		.isPremium = snap.is_premium,
		.unreadCount = snap.unread_count,
		.unreadMuted = snap.unread_muted,
	};
}

[[nodiscard]] ayu::ffi::FfiMessageRow ToFfiMessage(
		const BridgeMessageRow &row) {
	return ayu::ffi::FfiMessageRow{
		.dialog_id = row.dialogId,
		.msg_id = row.msgId,
		.sender_id = row.senderId,
		.reply_to_msg_id = row.replyToMsgId,
		.date_unix = row.dateUnix,
		.edit_date_unix = row.editDateUnix,
		.views = row.views,
		.flags = row.flags,
		.grouped_id = row.groupedId,
		.author_name = ToRustString(row.authorName),
		.text = ToRustString(row.text),
		.media_kind = row.mediaKind,
		.media_path = ToRustString(row.mediaPath),
	};
}

[[nodiscard]] BridgeMessageRow FromFfiMessage(
		const ayu::ffi::FfiMessageRow &row) {
	return BridgeMessageRow{
		.dialogId = row.dialog_id,
		.msgId = row.msg_id,
		.senderId = row.sender_id,
		.replyToMsgId = row.reply_to_msg_id,
		.dateUnix = row.date_unix,
		.editDateUnix = row.edit_date_unix,
		.views = row.views,
		.flags = row.flags,
		.groupedId = row.grouped_id,
		.authorName = FromRustString(row.author_name),
		.text = FromRustString(row.text),
		.mediaKind = row.media_kind,
		.mediaPath = FromRustString(row.media_path),
	};
}

[[nodiscard]] std::vector<BridgeMessageRow> ConvertMessageVec(
		const rust::Vec<ayu::ffi::FfiMessageRow> &rows) {
	auto out = std::vector<BridgeMessageRow>();
	out.reserve(rows.size());
	for (const auto &row : rows) {
		out.push_back(FromFfiMessage(row));
	}
	return out;
}

[[nodiscard]] ayu::ffi::FfiDialogRow ToFfiDialog(
		const BridgeDialogRow &row) {
	return ayu::ffi::FfiDialogRow{
		.dialog_id = row.dialogId,
		.peer_id = row.peerId,
		.folder_id = row.folderId,
		.title = ToRustString(row.title),
		.draft_text = ToRustString(row.draftText),
		.top_message_id = row.topMessageId,
		.read_inbox_max_id = row.readInboxMaxId,
		.read_outbox_max_id = row.readOutboxMaxId,
		.last_message_date = row.lastMessageDate,
		.unread_count = row.unreadCount,
		.unread_mentions_count = row.unreadMentionsCount,
		.flags = row.flags,
	};
}

[[nodiscard]] BridgeDialogRow FromFfiDialog(
		const ayu::ffi::FfiDialogRow &row) {
	return BridgeDialogRow{
		.dialogId = row.dialog_id,
		.peerId = row.peer_id,
		.folderId = row.folder_id,
		.title = FromRustString(row.title),
		.draftText = FromRustString(row.draft_text),
		.topMessageId = row.top_message_id,
		.readInboxMaxId = row.read_inbox_max_id,
		.readOutboxMaxId = row.read_outbox_max_id,
		.lastMessageDate = row.last_message_date,
		.unreadCount = row.unread_count,
		.unreadMentionsCount = row.unread_mentions_count,
		.flags = row.flags,
	};
}

[[nodiscard]] std::vector<BridgeDialogRow> ConvertDialogVec(
		const rust::Vec<ayu::ffi::FfiDialogRow> &rows) {
	auto out = std::vector<BridgeDialogRow>();
	out.reserve(rows.size());
	for (const auto &row : rows) {
		out.push_back(FromFfiDialog(row));
	}
	return out;
}

[[nodiscard]] BridgePtsOutcome FromFfiPtsOutcome(
		const ayu::ffi::FfiPtsOutcome &outcome) {
	return BridgePtsOutcome{
		.applied = outcome.applied,
		.gapDetected = outcome.gap_detected,
		.duplicate = outcome.duplicate,
		.currentPts = outcome.current_pts,
	};
}

[[nodiscard]] rust::Vec<std::uint8_t> ToRustBytesVec(const QByteArray &bytes) {
	auto out = rust::Vec<std::uint8_t>();
	out.reserve(std::size_t(bytes.size()));
	for (const auto byte : bytes) {
		out.push_back(static_cast<std::uint8_t>(byte));
	}
	return out;
}

[[nodiscard]] QByteArray FromRustBytesVec(
		const rust::Vec<std::uint8_t> &bytes) {
	return QByteArray(
		reinterpret_cast<const char*>(bytes.data()),
		int(bytes.size()));
}

[[nodiscard]] rust::Slice<const std::uint8_t> ToRustByteSlice(
		const QByteArray &bytes) {
	return rust::Slice<const std::uint8_t>(
		reinterpret_cast<const std::uint8_t*>(bytes.constData()),
		std::size_t(bytes.size()));
}

} // namespace

struct RustBridge::Impl {
	explicit Impl(rust::Box<ayu::ffi::AyuRustEngine> created)
	: engine(std::move(created)) {
	}

	rust::Box<ayu::ffi::AyuRustEngine> engine;
	QString lastError;
};

RustBridge::RustBridge(std::unique_ptr<Impl> impl)
: _impl(std::move(impl)) {
}

RustBridge::~RustBridge() = default;

std::unique_ptr<RustBridge> RustBridge::Create(
		const QString &dbPath,
		std::size_t mediaCacheLimitMb) {
	try {
		const auto utf8 = dbPath.toUtf8();
		auto engine = ayu::ffi::create_ayu_engine(
			rust::Str(utf8.constData(), std::size_t(utf8.size())),
			mediaCacheLimitMb);
		auto impl = std::make_unique<Impl>(std::move(engine));
		return std::unique_ptr<RustBridge>(new RustBridge(std::move(impl)));
	} catch (const rust::Error &) {
		return nullptr;
	}
}

void RustBridge::InitializeGlobal(
		const QString &dbPath,
		std::size_t mediaCacheLimitMb) {
	if (!GlobalBridge) {
		GlobalBridge = Create(dbPath, mediaCacheLimitMb);
	}
}

RustBridge *RustBridge::Global() {
	return GlobalBridge.get();
}

bool RustBridge::isReady() const {
	if (!_impl) {
		return false;
	}
	const auto status = _impl->engine->health_check();
	if (!status.ok) {
		_impl->lastError = FromRustString(status.error_message);
	}
	return status.ok;
}

QString RustBridge::lastError() const {
	return _impl
		? _impl->lastError
		: QString::fromUtf8("RustBridge is uninitialized");
}

bool RustBridge::registerAccount(
		std::uint64_t accountId,
		std::uint64_t userId,
		bool isTestDc) {
	if (!_impl) {
		return false;
	}
	try {
		_impl->engine->register_account(accountId, userId, isTestDc);
		_impl->lastError.clear();
		return true;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return false;
	}
}

bool RustBridge::activateAccount(std::uint64_t accountId) {
	if (!_impl) {
		return false;
	}
	try {
		_impl->engine->activate_account(accountId);
		_impl->lastError.clear();
		return true;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return false;
	}
}

bool RustBridge::removeAccount(std::uint64_t accountId) {
	if (!_impl) {
		return false;
	}
	try {
		_impl->engine->remove_account(accountId);
		_impl->lastError.clear();
		return true;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return false;
	}
}

auto RustBridge::importMtpAuthorization(
		std::uint64_t accountId,
		const QByteArray &serialized)
-> std::optional<BridgeAccountSnapshot> {
	if (!_impl) {
		return std::nullopt;
	}
	try {
		const auto snap = _impl->engine->import_mtp_authorization(
			accountId,
			rust::Slice<const std::uint8_t>(
				reinterpret_cast<const std::uint8_t*>(serialized.constData()),
				std::size_t(serialized.size())));
		_impl->lastError.clear();
		return FromFfiSnapshot(snap);
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return std::nullopt;
	}
}

QByteArray RustBridge::exportMtpAuthorization(std::uint64_t accountId) {
	if (!_impl) {
		return QByteArray();
	}
	try {
		const auto bytes = _impl->engine->export_mtp_authorization(accountId);
		_impl->lastError.clear();
		return QByteArray(
			reinterpret_cast<const char*>(bytes.data()),
			int(bytes.size()));
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return QByteArray();
	}
}

bool RustBridge::completeAuthorization(
		std::uint64_t accountId,
		std::uint64_t userId,
		std::int32_t mainDcId,
		bool isPremium) {
	if (!_impl) {
		return false;
	}
	try {
		_impl->engine->complete_authorization(
			accountId,
			userId,
			mainDcId,
			isPremium);
		_impl->lastError.clear();
		return true;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return false;
	}
}

bool RustBridge::beginLogout(std::uint64_t accountId) {
	if (!_impl) {
		return false;
	}
	try {
		_impl->engine->begin_logout(accountId);
		_impl->lastError.clear();
		return true;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return false;
	}
}

bool RustBridge::finishLogout(std::uint64_t accountId) {
	if (!_impl) {
		return false;
	}
	try {
		_impl->engine->finish_logout(accountId);
		_impl->lastError.clear();
		return true;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return false;
	}
}

bool RustBridge::forceLogout(std::uint64_t accountId) {
	if (!_impl) {
		return false;
	}
	try {
		_impl->engine->force_logout(accountId);
		_impl->lastError.clear();
		return true;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return false;
	}
}

bool RustBridge::pushLoginCode(
		std::uint64_t accountId,
		const QString &code) {
	if (!_impl) {
		return false;
	}
	try {
		const auto utf8 = code.toUtf8();
		_impl->engine->push_login_code(
			accountId,
			rust::Str(utf8.constData(), std::size_t(utf8.size())));
		_impl->lastError.clear();
		return true;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return false;
	}
}

QString RustBridge::takeLoginCode(std::uint64_t accountId) {
	if (!_impl) {
		return QString();
	}
	try {
		const auto code = _impl->engine->take_login_code(accountId);
		_impl->lastError.clear();
		return FromRustString(code);
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return QString();
	}
}

bool RustBridge::setAccountUnreadState(
		std::uint64_t accountId,
		std::uint32_t unreadCount,
		bool muted) {
	if (!_impl) {
		return false;
	}
	try {
		_impl->engine->set_account_unread_state(
			accountId,
			unreadCount,
			muted);
		_impl->lastError.clear();
		return true;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return false;
	}
}

bool RustBridge::setAccountSeparateWindow(
		std::uint64_t accountId,
		bool hasWindow) {
	if (!_impl) {
		return false;
	}
	try {
		_impl->engine->set_account_separate_window(accountId, hasWindow);
		_impl->lastError.clear();
		return true;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return false;
	}
}

bool RustBridge::markAccountOnline(std::uint64_t accountId) {
	if (!_impl) {
		return false;
	}
	try {
		_impl->engine->mark_account_online(accountId);
		_impl->lastError.clear();
		return true;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return false;
	}
}

auto RustBridge::collectAccountsForOfflinePacket()
-> std::vector<std::uint64_t> {
	auto out = std::vector<std::uint64_t>();
	if (!_impl) {
		return out;
	}
	const auto ids = _impl->engine->collect_accounts_for_offline_packet();
	out.assign(ids.begin(), ids.end());
	return out;
}

BridgeUnreadBadge RustBridge::computeUnreadBadge() const {
	if (!_impl) {
		return BridgeUnreadBadge{};
	}
	const auto badge = _impl->engine->compute_unread_badge();
	return BridgeUnreadBadge{
		.totalUnread = badge.total_unread,
		.allMuted = badge.all_muted,
	};
}

auto RustBridge::orderedAccountIds(
		const std::vector<std::uint64_t> &orderUniqueIds) const
-> std::vector<std::uint64_t> {
	auto out = std::vector<std::uint64_t>();
	if (!_impl) {
		return out;
	}
	const auto ids = _impl->engine->ordered_account_ids(
		rust::Slice<const std::uint64_t>(
			orderUniqueIds.data(),
			orderUniqueIds.size()));
	out.assign(ids.begin(), ids.end());
	return out;
}

auto RustBridge::redundantAccountIds() const -> std::vector<std::uint64_t> {
	auto out = std::vector<std::uint64_t>();
	if (!_impl) {
		return out;
	}
	const auto ids = _impl->engine->redundant_account_ids();
	out.assign(ids.begin(), ids.end());
	return out;
}

std::size_t RustBridge::maxAccountsLimit() const {
	return _impl ? _impl->engine->max_accounts_limit() : 0;
}

std::size_t RustBridge::authedAccountsCount() const {
	return _impl ? _impl->engine->authed_accounts_count() : 0;
}

auto RustBridge::accountSnapshot(std::uint64_t accountId) const
-> std::optional<BridgeAccountSnapshot> {
	if (!_impl) {
		return std::nullopt;
	}
	try {
		const auto snap = _impl->engine->get_account_snapshot(accountId);
		return FromFfiSnapshot(snap);
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return std::nullopt;
	}
}

bool RustBridge::setGhostConfig(
		std::uint64_t accountId,
		const BridgeGhostConfig &config) {
	if (!_impl) {
		return false;
	}
	try {
		_impl->engine->set_ghost_config(
			accountId,
			ayu::ffi::FfiGhostConfig{
				.ghost_mode_active = config.ghostModeActive,
				.send_read_messages = config.sendReadMessages,
				.send_read_stories = config.sendReadStories,
				.send_online_packets = config.sendOnlinePackets,
				.send_upload_progress = config.sendUploadProgress,
				.send_offline_after_online = config.sendOfflineAfterOnline,
				.mark_read_after_action = config.markReadAfterAction,
				.use_scheduled_messages = config.useScheduledMessages,
				.send_without_sound = config.sendWithoutSound,
			});
		_impl->lastError.clear();
		return true;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return false;
	}
}

auto RustBridge::ghostConfig(std::uint64_t accountId)
-> std::optional<BridgeGhostConfig> {
	if (!_impl) {
		return std::nullopt;
	}
	try {
		const auto cfg = _impl->engine->get_ghost_config(accountId);
		_impl->lastError.clear();
		return BridgeGhostConfig{
			.ghostModeActive = cfg.ghost_mode_active,
			.sendReadMessages = cfg.send_read_messages,
			.sendReadStories = cfg.send_read_stories,
			.sendOnlinePackets = cfg.send_online_packets,
			.sendUploadProgress = cfg.send_upload_progress,
			.sendOfflineAfterOnline = cfg.send_offline_after_online,
			.markReadAfterAction = cfg.mark_read_after_action,
			.useScheduledMessages = cfg.use_scheduled_messages,
			.sendWithoutSound = cfg.send_without_sound,
		};
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return std::nullopt;
	}
}

bool RustBridge::shouldSendReadAck(std::uint64_t accountId) const {
	return !_impl || _impl->engine->should_send_read_ack(accountId);
}

bool RustBridge::shouldSendOnlinePacket(std::uint64_t accountId) const {
	return !_impl || _impl->engine->should_send_online_packet(accountId);
}

bool RustBridge::upsertMessage(
		std::uint64_t accountId,
		const BridgeMessageRow &row) {
	if (!_impl) {
		return false;
	}
	try {
		_impl->engine->upsert_message(accountId, ToFfiMessage(row));
		_impl->lastError.clear();
		return true;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return false;
	}
}

std::size_t RustBridge::upsertMessageBatch(
		std::uint64_t accountId,
		const std::vector<BridgeMessageRow> &rows) {
	if (!_impl || rows.empty()) {
		return 0;
	}
	try {
		auto ffiRows = std::vector<ayu::ffi::FfiMessageRow>();
		ffiRows.reserve(rows.size());
		for (const auto &row : rows) {
			ffiRows.push_back(ToFfiMessage(row));
		}
		const auto written = _impl->engine->upsert_message_batch(
			accountId,
			rust::Slice<const ayu::ffi::FfiMessageRow>(
				ffiRows.data(),
				ffiRows.size()));
		_impl->lastError.clear();
		return written;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return 0;
	}
}

auto RustBridge::loadViewportMessages(
		std::uint64_t accountId,
		std::int64_t dialogId,
		std::int32_t centerMsgId,
		std::size_t limit)
-> std::vector<BridgeMessageRow> {
	if (!_impl) {
		return {};
	}
	try {
		const auto rows = _impl->engine->load_viewport_messages(
			accountId,
			dialogId,
			centerMsgId,
			limit);
		_impl->lastError.clear();
		return ConvertMessageVec(rows);
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return {};
	}
}

auto RustBridge::scrollViewportOlder(
		std::uint64_t accountId,
		std::int64_t dialogId,
		std::size_t batchSize)
-> std::vector<BridgeMessageRow> {
	if (!_impl) {
		return {};
	}
	try {
		const auto rows = _impl->engine->scroll_viewport_older(
			accountId,
			dialogId,
			batchSize);
		_impl->lastError.clear();
		return ConvertMessageVec(rows);
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return {};
	}
}

auto RustBridge::scrollViewportNewer(
		std::uint64_t accountId,
		std::int64_t dialogId,
		std::size_t batchSize)
-> std::vector<BridgeMessageRow> {
	if (!_impl) {
		return {};
	}
	try {
		const auto rows = _impl->engine->scroll_viewport_newer(
			accountId,
			dialogId,
			batchSize);
		_impl->lastError.clear();
		return ConvertMessageVec(rows);
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return {};
	}
}

std::size_t RustBridge::deleteMessages(
		std::uint64_t accountId,
		std::int64_t dialogId,
		const std::vector<std::int32_t> &msgIds) {
	if (!_impl || msgIds.empty()) {
		return 0;
	}
	try {
		const auto removed = _impl->engine->delete_messages(
			accountId,
			dialogId,
			rust::Slice<const std::int32_t>(
				msgIds.data(),
				msgIds.size()));
		_impl->lastError.clear();
		return removed;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return 0;
	}
}

std::size_t RustBridge::clearDialogHistoryUpTo(
		std::uint64_t accountId,
		std::int64_t dialogId,
		std::int32_t maxMsgId) {
	if (!_impl) {
		return 0;
	}
	try {
		const auto removed = _impl->engine->clear_dialog_history_up_to(
			accountId,
			dialogId,
			maxMsgId);
		_impl->lastError.clear();
		return removed;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return 0;
	}
}

void RustBridge::unloadDialogViewport(
		std::uint64_t accountId,
		std::int64_t dialogId) {
	if (_impl) {
		_impl->engine->unload_dialog_viewport(accountId, dialogId);
	}
}

auto RustBridge::searchDialogMessages(
		std::uint64_t accountId,
		std::int64_t dialogId,
		const QString &query,
		std::size_t limit)
-> std::vector<BridgeMessageRow> {
	if (!_impl) {
		return {};
	}
	try {
		const auto utf8 = query.toUtf8();
		const auto rows = _impl->engine->search_dialog_messages(
			accountId,
			dialogId,
			rust::Str(utf8.constData(), std::size_t(utf8.size())),
			limit);
		_impl->lastError.clear();
		return ConvertMessageVec(rows);
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return {};
	}
}

bool RustBridge::upsertDialog(
		std::uint64_t accountId,
		const BridgeDialogRow &row) {
	if (!_impl) {
		return false;
	}
	try {
		_impl->engine->upsert_dialog(accountId, ToFfiDialog(row));
		_impl->lastError.clear();
		return true;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return false;
	}
}

std::size_t RustBridge::upsertDialogBatch(
		std::uint64_t accountId,
		const std::vector<BridgeDialogRow> &rows) {
	if (!_impl || rows.empty()) {
		return 0;
	}
	try {
		auto ffiRows = std::vector<ayu::ffi::FfiDialogRow>();
		ffiRows.reserve(rows.size());
		for (const auto &row : rows) {
			ffiRows.push_back(ToFfiDialog(row));
		}
		const auto written = _impl->engine->upsert_dialog_batch(
			accountId,
			rust::Slice<const ayu::ffi::FfiDialogRow>(
				ffiRows.data(),
				ffiRows.size()));
		_impl->lastError.clear();
		return written;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return 0;
	}
}

bool RustBridge::updateDialogReadState(
		std::uint64_t accountId,
		std::int64_t dialogId,
		std::int32_t readInboxMaxId,
		std::uint32_t unreadCount) {
	if (!_impl) {
		return false;
	}
	try {
		const auto updated = _impl->engine->update_dialog_read_state(
			accountId,
			dialogId,
			readInboxMaxId,
			unreadCount);
		_impl->lastError.clear();
		return updated;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return false;
	}
}

bool RustBridge::removeDialog(
		std::uint64_t accountId,
		std::int64_t dialogId) {
	if (!_impl) {
		return false;
	}
	try {
		const auto removed = _impl->engine->remove_dialog(
			accountId,
			dialogId);
		_impl->lastError.clear();
		return removed;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return false;
	}
}

auto RustBridge::loadDialogSlice(
		std::uint64_t accountId,
		std::int32_t folderId,
		std::size_t offset,
		std::size_t limit)
-> std::vector<BridgeDialogRow> {
	if (!_impl) {
		return {};
	}
	try {
		const auto rows = _impl->engine->load_dialog_slice(
			accountId,
			folderId,
			offset,
			limit);
		_impl->lastError.clear();
		return ConvertDialogVec(rows);
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return {};
	}
}

auto RustBridge::searchDialogs(
		std::uint64_t accountId,
		const QString &query,
		std::size_t limit)
-> std::vector<BridgeDialogRow> {
	if (!_impl) {
		return {};
	}
	try {
		const auto utf8 = query.toUtf8();
		const auto rows = _impl->engine->search_dialogs(
			accountId,
			rust::Str(utf8.constData(), std::size_t(utf8.size())),
			limit);
		_impl->lastError.clear();
		return ConvertDialogVec(rows);
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return {};
	}
}

bool RustBridge::recordDeletedMessage(
		std::uint64_t accountId,
		const BridgeMessageRow &row) {
	if (!_impl) {
		return false;
	}
	try {
		_impl->engine->record_deleted_message(accountId, ToFfiMessage(row));
		_impl->lastError.clear();
		return true;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return false;
	}
}

auto RustBridge::loadDeletedMessages(
		std::uint64_t accountId,
		std::int64_t dialogId,
		std::size_t limit)
-> std::vector<BridgeMessageRow> {
	if (!_impl) {
		return {};
	}
	try {
		const auto rows = _impl->engine->load_deleted_messages(
			accountId,
			dialogId,
			limit);
		_impl->lastError.clear();
		return ConvertMessageVec(rows);
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return {};
	}
}

bool RustBridge::hasDeletedMessages(
		std::uint64_t accountId,
		std::int64_t dialogId) const {
	return _impl
		? _impl->engine->has_deleted_messages(accountId, dialogId)
		: false;
}

bool RustBridge::recordEditedMessage(
		std::uint64_t accountId,
		const BridgeMessageRow &row) {
	if (!_impl) {
		return false;
	}
	try {
		_impl->engine->record_edited_message(accountId, ToFfiMessage(row));
		_impl->lastError.clear();
		return true;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return false;
	}
}

auto RustBridge::loadEditedMessages(
		std::uint64_t accountId,
		std::int64_t dialogId,
		std::int32_t msgId,
		std::size_t limit)
-> std::vector<BridgeMessageRow> {
	if (!_impl) {
		return {};
	}
	try {
		const auto rows = _impl->engine->load_edited_messages(
			accountId,
			dialogId,
			msgId,
			limit);
		_impl->lastError.clear();
		return ConvertMessageVec(rows);
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return {};
	}
}

bool RustBridge::hasEditedRevisions(
		std::uint64_t accountId,
		std::int64_t dialogId,
		std::int32_t msgId) const {
	return _impl
		? _impl->engine->has_edited_revisions(accountId, dialogId, msgId)
		: false;
}

bool RustBridge::putMediaCache(
		std::uint64_t key,
		const QByteArray &data) {
	if (!_impl) {
		return false;
	}
	try {
		_impl->engine->put_media_cache(
			key,
			rust::Slice<const std::uint8_t>(
				reinterpret_cast<const std::uint8_t*>(data.constData()),
				std::size_t(data.size())));
		_impl->lastError.clear();
		return true;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return false;
	}
}

bool RustBridge::putScopedMediaCache(
		std::uint64_t accountId,
		std::uint8_t kind,
		std::uint64_t resourceId,
		const QByteArray &data) {
	if (!_impl) {
		return false;
	}
	try {
		_impl->engine->put_scoped_media_cache(
			accountId,
			kind,
			resourceId,
			rust::Slice<const std::uint8_t>(
				reinterpret_cast<const std::uint8_t*>(data.constData()),
				std::size_t(data.size())));
		_impl->lastError.clear();
		return true;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return false;
	}
}

QByteArray RustBridge::getScopedMediaCache(
		std::uint64_t accountId,
		std::uint8_t kind,
		std::uint64_t resourceId) const {
	if (!_impl) {
		return QByteArray();
	}
	const auto bytes = _impl->engine->get_scoped_media_cache(
		accountId,
		kind,
		resourceId);
	return QByteArray(
		reinterpret_cast<const char*>(bytes.data()),
		int(bytes.size()));
}

std::size_t RustBridge::evictAccountMediaCache(
		std::uint64_t accountId,
		bool heavyOnly) {
	return _impl
		? _impl->engine->evict_account_media_cache(accountId, heavyOnly)
		: 0;
}

std::size_t RustBridge::evictInactiveCaches() {
	return _impl ? _impl->engine->evict_inactive_caches() : 0;
}

BridgeMemoryStats RustBridge::memoryStats() const {
	if (!_impl) {
		return BridgeMemoryStats{};
	}
	const auto stats = _impl->engine->get_memory_stats();
	return BridgeMemoryStats{
		.activeAccounts = stats.active_accounts,
		.residentWindows = stats.resident_windows,
		.cachedWindowMessages = stats.cached_window_messages,
		.cachedWindowBytes = stats.cached_window_bytes,
		.residentDialogWindows = stats.resident_dialog_windows,
		.cachedDialogRows = stats.cached_dialog_rows,
		.mediaCacheEntries = stats.media_cache_entries,
		.mediaCacheBytes = stats.media_cache_bytes,
		.mediaCacheLimitBytes = stats.media_cache_limit_bytes,
		.processRssBytes = stats.process_rss_bytes,
	};
}

bool RustBridge::initSyncState(
		std::uint64_t accountId,
		std::int32_t pts,
		std::int32_t qts,
		std::int32_t date,
		std::int32_t seq) {
	if (!_impl) {
		return false;
	}
	try {
		_impl->engine->init_sync_state(accountId, pts, qts, date, seq);
		_impl->lastError.clear();
		return true;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return false;
	}
}

auto RustBridge::syncState(std::uint64_t accountId) const
-> std::optional<BridgeSyncState> {
	if (!_impl) {
		return std::nullopt;
	}
	try {
		const auto st = _impl->engine->get_sync_state(accountId);
		_impl->lastError.clear();
		return BridgeSyncState{
			.pts = st.pts,
			.qts = st.qts,
			.date = st.date,
			.seq = st.seq,
		};
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return std::nullopt;
	}
}

auto RustBridge::applyPtsUpdate(
		std::uint64_t accountId,
		std::uint64_t channelId,
		std::int32_t pts,
		std::int32_t ptsCount)
-> std::optional<BridgePtsOutcome> {
	if (!_impl) {
		return std::nullopt;
	}
	try {
		const auto res = _impl->engine->apply_pts_update(
			accountId,
			channelId,
			pts,
			ptsCount);
		_impl->lastError.clear();
		return FromFfiPtsOutcome(res);
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return std::nullopt;
	}
}

std::int32_t RustBridge::queueOutgoingMessage(
		std::uint64_t accountId,
		std::uint64_t randomId,
		const BridgeMessageRow &row) {
	if (!_impl) {
		return 0;
	}
	try {
		const auto assignedId = _impl->engine->queue_outgoing_message(
			accountId,
			randomId,
			ToFfiMessage(row));
		_impl->lastError.clear();
		return assignedId;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return 0;
	}
}

bool RustBridge::confirmOutgoingMessage(
		std::uint64_t accountId,
		std::uint64_t randomId,
		std::int32_t serverMsgId,
		std::int32_t dateUnix,
		std::int32_t pts,
		std::int32_t ptsCount) {
	if (!_impl) {
		return false;
	}
	try {
		const auto ok = _impl->engine->confirm_outgoing_message(
			accountId,
			randomId,
			serverMsgId,
			dateUnix,
			pts,
			ptsCount);
		_impl->lastError.clear();
		return ok;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return false;
	}
}

auto RustBridge::receiveIncomingMessage(
		std::uint64_t accountId,
		std::uint64_t channelId,
		std::int32_t pts,
		std::int32_t ptsCount,
		const BridgeMessageRow &row)
-> std::optional<BridgePtsOutcome> {
	if (!_impl) {
		return std::nullopt;
	}
	try {
		const auto res = _impl->engine->receive_incoming_message(
			accountId,
			channelId,
			pts,
			ptsCount,
			ToFfiMessage(row));
		_impl->lastError.clear();
		return FromFfiPtsOutcome(res);
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return std::nullopt;
	}
}

std::uint64_t RustBridge::enqueueFileDownload(
		std::uint64_t accountId,
		std::uint16_t dcId,
		std::uint64_t fileId,
		std::uint64_t totalBytes,
		std::int32_t priority) {
	if (!_impl) {
		return 0;
	}
	try {
		const auto id = _impl->engine->enqueue_file_download(
			accountId,
			dcId,
			fileId,
			totalBytes,
			priority);
		_impl->lastError.clear();
		return id;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return 0;
	}
}

std::uint64_t RustBridge::enqueueFileUpload(
		std::uint64_t accountId,
		std::uint16_t dcId,
		std::uint64_t fileId,
		std::uint64_t totalBytes) {
	if (!_impl) {
		return 0;
	}
	try {
		const auto id = _impl->engine->enqueue_file_upload(
			accountId,
			dcId,
			fileId,
			totalBytes);
		_impl->lastError.clear();
		return id;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return 0;
	}
}

auto RustBridge::advanceFileTransfer(
		std::uint64_t taskId,
		std::size_t chunkBytes)
-> std::optional<BridgeTransferProgress> {
	if (!_impl) {
		return std::nullopt;
	}
	try {
		const auto p = _impl->engine->advance_file_transfer(
			taskId,
			chunkBytes);
		_impl->lastError.clear();
		return BridgeTransferProgress{
			.taskId = p.task_id,
			.fileId = p.file_id,
			.transferredBytes = p.transferred_bytes,
			.totalBytes = p.total_bytes,
			.partSize = p.part_size,
			.partsTotal = p.parts_total,
			.partsCompleted = p.parts_completed,
			.state = p.state,
			.isBigFile = p.is_big_file,
		};
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return std::nullopt;
	}
}

bool RustBridge::cancelFileTransfer(std::uint64_t taskId) {
	return _impl ? _impl->engine->cancel_file_transfer(taskId) : false;
}

std::size_t RustBridge::activeTransferInflightBytes() const {
	return _impl ? _impl->engine->active_transfer_inflight_bytes() : 0;
}

auto RustBridge::queryDeletedMessages(
		std::uint64_t accountId,
		std::int64_t dialogId,
		std::int64_t topicId,
		std::int32_t minId,
		std::int32_t maxId,
		std::size_t limit,
		const QString &searchQuery)
-> std::vector<BridgeMessageRow> {
	if (!_impl) {
		return {};
	}
	try {
		const auto utf8 = searchQuery.toUtf8();
		const auto rows = _impl->engine->query_deleted_messages(
			accountId,
			dialogId,
			topicId,
			minId,
			maxId,
			limit,
			rust::Str(utf8.constData(), std::size_t(utf8.size())));
		_impl->lastError.clear();
		return ConvertMessageVec(rows);
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return {};
	}
}

std::size_t RustBridge::removeDeletedMessage(
		std::uint64_t accountId,
		std::int64_t dialogId,
		std::int32_t msgId) {
	if (!_impl) {
		return 0;
	}
	try {
		const auto removed = _impl->engine->remove_deleted_message(
			accountId,
			dialogId,
			msgId);
		_impl->lastError.clear();
		return removed;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return 0;
	}
}

std::size_t RustBridge::clearDeletedMessages(
		std::uint64_t accountId,
		std::int64_t dialogId,
		std::int64_t topicId) {
	if (!_impl) {
		return 0;
	}
	try {
		const auto cleared = _impl->engine->clear_deleted_messages(
			accountId,
			dialogId,
			topicId);
		_impl->lastError.clear();
		return cleared;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return 0;
	}
}

auto RustBridge::queryEditedMessages(
		std::uint64_t accountId,
		std::int64_t dialogId,
		std::int32_t msgId,
		std::int64_t minId,
		std::int64_t maxId,
		std::size_t limit)
-> std::vector<BridgeMessageRow> {
	if (!_impl) {
		return {};
	}
	try {
		const auto rows = _impl->engine->query_edited_messages(
			accountId,
			dialogId,
			msgId,
			minId,
			maxId,
			limit);
		_impl->lastError.clear();
		return ConvertMessageVec(rows);
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return {};
	}
}

bool RustBridge::upsertRegexFilter(const BridgeRegexFilter &filter) {
	if (!_impl) {
		return false;
	}
	try {
		_impl->engine->upsert_regex_filter(ayu::ffi::FfiRegexFilter{
			.id = ToRustBytesVec(filter.id),
			.pattern = ToRustString(filter.pattern),
			.enabled = filter.enabled,
			.reversed = filter.reversed,
			.case_insensitive = filter.caseInsensitive,
			.has_dialog_id = filter.hasDialogId,
			.dialog_id = filter.dialogId,
		});
		_impl->lastError.clear();
		return true;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return false;
	}
}

bool RustBridge::deleteRegexFilter(const QByteArray &id) {
	if (!_impl) {
		return false;
	}
	try {
		const auto deleted = _impl->engine->delete_regex_filter(
			ToRustByteSlice(id));
		_impl->lastError.clear();
		return deleted;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return false;
	}
}

std::size_t RustBridge::clearRegexFilters() {
	if (!_impl) {
		return 0;
	}
	try {
		const auto cleared = _impl->engine->clear_regex_filters();
		_impl->lastError.clear();
		return cleared;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return 0;
	}
}

auto RustBridge::listRegexFilters() -> std::vector<BridgeRegexFilter> {
	if (!_impl) {
		return {};
	}
	try {
		const auto rows = _impl->engine->list_regex_filters();
		auto out = std::vector<BridgeRegexFilter>();
		out.reserve(rows.size());
		for (const auto &row : rows) {
			out.push_back(BridgeRegexFilter{
				.id = FromRustBytesVec(row.id),
				.pattern = FromRustString(row.pattern),
				.enabled = row.enabled,
				.reversed = row.reversed,
				.caseInsensitive = row.case_insensitive,
				.hasDialogId = row.has_dialog_id,
				.dialogId = row.dialog_id,
			});
		}
		_impl->lastError.clear();
		return out;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return {};
	}
}

bool RustBridge::addRegexExclusion(
		std::int64_t dialogId,
		const QByteArray &filterId) {
	if (!_impl) {
		return false;
	}
	try {
		_impl->engine->add_regex_exclusion(
			dialogId,
			ToRustByteSlice(filterId));
		_impl->lastError.clear();
		return true;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return false;
	}
}

bool RustBridge::removeRegexExclusion(
		std::int64_t dialogId,
		const QByteArray &filterId) {
	if (!_impl) {
		return false;
	}
	try {
		const auto removed = _impl->engine->remove_regex_exclusion(
			dialogId,
			ToRustByteSlice(filterId));
		_impl->lastError.clear();
		return removed;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return false;
	}
}

std::size_t RustBridge::deleteExclusionsByFilterId(
		const QByteArray &filterId) {
	if (!_impl) {
		return 0;
	}
	try {
		const auto removed = _impl->engine->delete_exclusions_by_filter_id(
			ToRustByteSlice(filterId));
		_impl->lastError.clear();
		return removed;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return 0;
	}
}

std::size_t RustBridge::clearRegexExclusions() {
	if (!_impl) {
		return 0;
	}
	try {
		const auto cleared = _impl->engine->clear_regex_exclusions();
		_impl->lastError.clear();
		return cleared;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return 0;
	}
}

std::size_t RustBridge::rebuildFiltersCache() {
	if (!_impl) {
		return 0;
	}
	try {
		const auto count = _impl->engine->rebuild_filters_cache();
		_impl->lastError.clear();
		return count;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return 0;
	}
}

void RustBridge::setFiltersEnabled(
		bool enabled,
		bool enabledInChats,
		bool hideFromBlocked) {
	if (_impl) {
		_impl->engine->set_filters_enabled(
			enabled,
			enabledInChats,
			hideFromBlocked);
	}
}

void RustBridge::setShadowBanned(std::int64_t peerDialogId, bool banned) {
	if (_impl) {
		_impl->engine->set_shadow_banned(peerDialogId, banned);
	}
}

bool RustBridge::isShadowBanned(std::int64_t peerDialogId) const {
	return _impl ? _impl->engine->is_shadow_banned(peerDialogId) : false;
}

bool RustBridge::evaluateMessageFilter(
		std::int64_t dialogId,
		std::int32_t msgId,
		std::int64_t senderDialogId,
		bool isOutgoing,
		const QString &text) {
	if (!_impl) {
		return false;
	}
	const auto utf8 = text.toUtf8();
	return _impl->engine->evaluate_message_filter(
		dialogId,
		msgId,
		senderDialogId,
		isOutgoing,
		rust::Str(utf8.constData(), std::size_t(utf8.size())));
}

void RustBridge::invalidateFilteredMessage(
		std::int64_t dialogId,
		std::int32_t msgId) {
	if (_impl) {
		_impl->engine->invalidate_filtered_message(dialogId, msgId);
	}
}

bool RustBridge::hasFilteredMessages(std::int64_t dialogId) const {
	return _impl ? _impl->engine->has_filtered_messages(dialogId) : false;
}

bool RustBridge::toggleFilteredMessagesShown(std::int64_t dialogId) {
	return _impl
		? _impl->engine->toggle_filtered_messages_shown(dialogId)
		: false;
}

void RustBridge::hideMessage(std::int64_t dialogId, std::int32_t msgId) {
	if (_impl) {
		_impl->engine->hide_message(dialogId, msgId);
	}
}

bool RustBridge::isMessageHidden(
		std::int64_t dialogId,
		std::int32_t msgId) const {
	return _impl ? _impl->engine->is_message_hidden(dialogId, msgId) : false;
}

void RustBridge::armDisableGhostOnStoryClose(std::uint64_t accountId) {
	if (_impl) {
		_impl->engine->arm_disable_ghost_on_story_close(accountId);
	}
}

bool RustBridge::consumeDisableGhostOnStoryClose(std::uint64_t accountId) {
	return _impl
		? _impl->engine->consume_disable_ghost_on_story_close(accountId)
		: false;
}

auto RustBridge::listRegexExclusions()
-> std::vector<BridgeRegexExclusion> {
	if (!_impl) {
		return {};
	}
	try {
		const auto items = _impl->engine->list_regex_exclusions();
		auto out = std::vector<BridgeRegexExclusion>();
		out.reserve(items.size());
		for (const auto &item : items) {
			out.push_back(BridgeRegexExclusion{
				.dialogId = item.dialog_id,
				.filterId = QByteArray(
					reinterpret_cast<const char*>(item.filter_id.data()),
					int(item.filter_id.size())),
			});
		}
		_impl->lastError.clear();
		return out;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return {};
	}
}

std::int64_t RustBridge::insertDeletedRecord(
		std::int64_t userId,
		std::int64_t dialogId,
		std::int64_t topicId,
		std::int32_t msgId,
		const QString &text,
		const QByteArray &payload) {
	if (!_impl) {
		return 0;
	}
	try {
		const auto utf8 = text.toUtf8();
		const auto id = _impl->engine->insert_deleted_record(
			userId,
			dialogId,
			topicId,
			msgId,
			rust::Str(utf8.constData(), utf8.size()),
			rust::Slice<const std::uint8_t>(
				reinterpret_cast<const std::uint8_t*>(payload.constData()),
				payload.size()));
		_impl->lastError.clear();
		return id;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return 0;
	}
}

auto RustBridge::queryDeletedRecords(
		std::int64_t userId,
		std::int64_t dialogId,
		std::int64_t topicId,
		std::int64_t minId,
		std::int64_t maxId,
		std::size_t limit,
		const QString &searchQuery)
-> std::vector<BridgeRawRecord> {
	if (!_impl) {
		return {};
	}
	try {
		const auto utf8 = searchQuery.toUtf8();
		const auto rows = _impl->engine->query_deleted_records(
			userId,
			dialogId,
			topicId,
			minId,
			maxId,
			limit,
			rust::Str(utf8.constData(), utf8.size()));
		auto out = std::vector<BridgeRawRecord>();
		out.reserve(rows.size());
		for (const auto &row : rows) {
			out.push_back(BridgeRawRecord{
				.fakeId = row.fake_id,
				.payload = QByteArray(
					reinterpret_cast<const char*>(row.payload.data()),
					int(row.payload.size())),
			});
		}
		_impl->lastError.clear();
		return out;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return {};
	}
}

bool RustBridge::hasDeletedRecords(
		std::int64_t userId,
		std::int64_t dialogId,
		std::int64_t topicId) const {
	if (!_impl) {
		return false;
	}
	try {
		return _impl->engine->has_deleted_records(userId, dialogId, topicId);
	} catch (const rust::Error &) {
		return false;
	}
}

std::size_t RustBridge::removeDeletedRecord(
		std::int64_t userId,
		std::int64_t dialogId,
		std::int32_t msgId) {
	if (!_impl) {
		return 0;
	}
	try {
		const auto n = _impl->engine->remove_deleted_record(
			userId,
			dialogId,
			msgId);
		_impl->lastError.clear();
		return n;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return 0;
	}
}

std::size_t RustBridge::clearDeletedRecords(
		std::int64_t userId,
		std::int64_t dialogId,
		std::int64_t topicId) {
	if (!_impl) {
		return 0;
	}
	try {
		const auto n = _impl->engine->clear_deleted_records(
			userId,
			dialogId,
			topicId);
		_impl->lastError.clear();
		return n;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return 0;
	}
}

std::int64_t RustBridge::insertEditedRecord(
		std::int64_t userId,
		std::int64_t dialogId,
		std::int32_t msgId,
		const QByteArray &payload) {
	if (!_impl) {
		return 0;
	}
	try {
		const auto id = _impl->engine->insert_edited_record(
			userId,
			dialogId,
			msgId,
			rust::Slice<const std::uint8_t>(
				reinterpret_cast<const std::uint8_t*>(payload.constData()),
				payload.size()));
		_impl->lastError.clear();
		return id;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return 0;
	}
}

auto RustBridge::queryEditedRecords(
		std::int64_t userId,
		std::int64_t dialogId,
		std::int32_t msgId,
		std::int64_t minId,
		std::int64_t maxId,
		std::size_t limit)
-> std::vector<BridgeRawRecord> {
	if (!_impl) {
		return {};
	}
	try {
		const auto rows = _impl->engine->query_edited_records(
			userId,
			dialogId,
			msgId,
			minId,
			maxId,
			limit);
		auto out = std::vector<BridgeRawRecord>();
		out.reserve(rows.size());
		for (const auto &row : rows) {
			out.push_back(BridgeRawRecord{
				.fakeId = row.fake_id,
				.payload = QByteArray(
					reinterpret_cast<const char*>(row.payload.data()),
					int(row.payload.size())),
			});
		}
		_impl->lastError.clear();
		return out;
	} catch (const rust::Error &err) {
		_impl->lastError = QString::fromUtf8(err.what());
		return {};
	}
}

bool RustBridge::hasEditedRecords(
		std::int64_t userId,
		std::int64_t dialogId,
		std::int32_t msgId) const {
	if (!_impl) {
		return false;
	}
	try {
		return _impl->engine->has_edited_records(userId, dialogId, msgId);
	} catch (const rust::Error &) {
		return false;
	}
}

} // namespace Ayu::Ffi
