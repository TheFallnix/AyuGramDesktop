// This is the source code of AyuGram for Desktop.
//
// We do not and cannot prevent the use of our code,
// but be respectful and credit the original author.
//
// Copyright @Radolyn, 2026
#include "ayu/data/ayu_database.h"

#include "ayu/data/entities.h"
#include "ayu/ffi/ayu_rust_bridge.h"
#include "base/unixtime.h"

#include <QtCore/QByteArray>
#include <QtCore/QDataStream>
#include <QtCore/QFile>
#include <QtCore/QIODevice>
#include <algorithm>

namespace AyuDatabase {
namespace {

constexpr auto kRecordFormatVersion = qint32(1);

[[nodiscard]] QByteArray ToQByteArray(const std::vector<char> &bytes) {
	return QByteArray(bytes.data(), int(bytes.size()));
}

[[nodiscard]] std::vector<char> FromQByteArray(const QByteArray &bytes) {
	return std::vector<char>(bytes.constBegin(), bytes.constEnd());
}

void WriteString(QDataStream &out, const std::string &value) {
	out << QByteArray(value.data(), int(value.size()));
}

[[nodiscard]] std::string ReadString(QDataStream &in) {
	auto raw = QByteArray();
	in >> raw;
	return std::string(raw.constData(), std::size_t(raw.size()));
}

void WriteBytes(QDataStream &out, const std::vector<char> &value) {
	out << ToQByteArray(value);
}

[[nodiscard]] std::vector<char> ReadBytes(QDataStream &in) {
	auto raw = QByteArray();
	in >> raw;
	return FromQByteArray(raw);
}

[[nodiscard]] QByteArray SerializeMessage(const AyuMessageBase &m) {
	auto buf = QByteArray();
	auto out = QDataStream(&buf, QIODevice::WriteOnly);
	out.setVersion(QDataStream::Qt_6_0);
	out << kRecordFormatVersion
		<< qint64(m.userId)
		<< qint64(m.dialogId)
		<< qint64(m.groupedId)
		<< qint64(m.peerId)
		<< qint64(m.fromId)
		<< qint64(m.topicId)
		<< qint32(m.messageId)
		<< qint32(m.date)
		<< qint32(m.flags)
		<< qint32(m.editDate)
		<< qint32(m.views)
		<< qint32(m.fwdFlags)
		<< qint64(m.fwdFromId)
		<< qint32(m.fwdDate)
		<< qint32(m.replyFlags)
		<< qint32(m.replyMessageId)
		<< qint64(m.replyPeerId)
		<< qint32(m.replyTopId)
		<< m.replyForumTopic
		<< qint32(m.entityCreateDate)
		<< qint32(m.documentType);
	WriteString(out, m.fwdName);
	WriteString(out, m.fwdPostAuthor);
	WriteString(out, m.postAuthor);
	WriteBytes(out, m.replySerialized);
	WriteBytes(out, m.replyMarkupSerialized);
	WriteString(out, m.text);
	WriteBytes(out, m.textEntities);
	WriteString(out, m.mediaPath);
	WriteString(out, m.hqThumbPath);
	WriteBytes(out, m.documentSerialized);
	WriteBytes(out, m.thumbsSerialized);
	WriteBytes(out, m.documentAttributesSerialized);
	WriteString(out, m.mimeType);
	return buf;
}

template <typename T>
[[nodiscard]] bool DeserializeMessage(
		std::int64_t fakeId,
		const QByteArray &payload,
		T &m) {
	auto in = QDataStream(payload);
	in.setVersion(QDataStream::Qt_6_0);
	auto version = qint32(0);
	in >> version;
	if (version != kRecordFormatVersion) {
		return false;
	}
	auto userId = qint64(0);
	auto dialogId = qint64(0);
	auto groupedId = qint64(0);
	auto peerId = qint64(0);
	auto fromId = qint64(0);
	auto topicId = qint64(0);
	auto messageId = qint32(0);
	auto date = qint32(0);
	auto flags = qint32(0);
	auto editDate = qint32(0);
	auto views = qint32(0);
	auto fwdFlags = qint32(0);
	auto fwdFromId = qint64(0);
	auto fwdDate = qint32(0);
	auto replyFlags = qint32(0);
	auto replyMessageId = qint32(0);
	auto replyPeerId = qint64(0);
	auto replyTopId = qint32(0);
	auto replyForumTopic = false;
	auto entityCreateDate = qint32(0);
	auto documentType = qint32(0);
	in >> userId >> dialogId >> groupedId >> peerId >> fromId >> topicId
		>> messageId >> date >> flags >> editDate >> views >> fwdFlags
		>> fwdFromId >> fwdDate >> replyFlags >> replyMessageId
		>> replyPeerId >> replyTopId >> replyForumTopic >> entityCreateDate
		>> documentType;
	m.fakeId = ID(fakeId);
	m.userId = ID(userId);
	m.dialogId = ID(dialogId);
	m.groupedId = ID(groupedId);
	m.peerId = ID(peerId);
	m.fromId = ID(fromId);
	m.topicId = ID(topicId);
	m.messageId = int(messageId);
	m.date = int(date);
	m.flags = int(flags);
	m.editDate = int(editDate);
	m.views = int(views);
	m.fwdFlags = int(fwdFlags);
	m.fwdFromId = ID(fwdFromId);
	m.fwdDate = int(fwdDate);
	m.replyFlags = int(replyFlags);
	m.replyMessageId = int(replyMessageId);
	m.replyPeerId = ID(replyPeerId);
	m.replyTopId = int(replyTopId);
	m.replyForumTopic = replyForumTopic;
	m.entityCreateDate = int(entityCreateDate);
	m.documentType = int(documentType);
	m.fwdName = ReadString(in);
	m.fwdPostAuthor = ReadString(in);
	m.postAuthor = ReadString(in);
	m.replySerialized = ReadBytes(in);
	m.replyMarkupSerialized = ReadBytes(in);
	m.text = ReadString(in);
	m.textEntities = ReadBytes(in);
	m.mediaPath = ReadString(in);
	m.hqThumbPath = ReadString(in);
	m.documentSerialized = ReadBytes(in);
	m.thumbsSerialized = ReadBytes(in);
	m.documentAttributesSerialized = ReadBytes(in);
	m.mimeType = ReadString(in);
	return (in.status() == QDataStream::Ok);
}

[[nodiscard]] Ayu::Ffi::BridgeRegexFilter ToBridgeFilter(
		const RegexFilter &filter) {
	return Ayu::Ffi::BridgeRegexFilter{
		.id = ToQByteArray(filter.id),
		.pattern = QString::fromStdString(filter.text),
		.enabled = filter.enabled,
		.reversed = filter.reversed,
		.caseInsensitive = filter.caseInsensitive,
		.hasDialogId = filter.dialogId.has_value(),
		.dialogId = filter.dialogId ? std::int64_t(*filter.dialogId) : 0,
	};
}

[[nodiscard]] RegexFilter FromBridgeFilter(
		const Ayu::Ffi::BridgeRegexFilter &filter) {
	auto out = RegexFilter();
	out.id = FromQByteArray(filter.id);
	out.text = filter.pattern.toStdString();
	out.enabled = filter.enabled;
	out.reversed = filter.reversed;
	out.caseInsensitive = filter.caseInsensitive;
	if (filter.hasDialogId) {
		out.dialogId = ID(filter.dialogId);
	}
	return out;
}

} // namespace

void moveCurrentDatabase() {
	const auto time = base::unixtime::now();
	const auto renameIf = [&](const QString &base, const QString &suffix) {
		const auto path = base + suffix;
		if (QFile::exists(path)) {
			QFile::rename(
				path,
				u"%1_%2%3"_q.arg(base).arg(time).arg(suffix));
		}
	};
	renameIf(u"./tdata/ayu_rust.db"_q, QString());
	renameIf(u"./tdata/ayu_rust.db"_q, u"-shm"_q);
	renameIf(u"./tdata/ayu_rust.db"_q, u"-wal"_q);
}

void initialize() {
	if (!Ayu::Ffi::RustBridge::Global()) {
		Ayu::Ffi::RustBridge::InitializeGlobal(u"./tdata/ayu_rust.db"_q, 4);
	}
}

void addEditedMessage(const EditedMessage &message) {
	if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
		bridge->insertEditedRecord(
			std::int64_t(message.userId),
			std::int64_t(message.dialogId),
			std::int32_t(message.messageId),
			SerializeMessage(message));
	}
}

std::vector<EditedMessage> getEditedMessages(
		ID userId,
		ID dialogId,
		ID messageId,
		ID minId,
		ID maxId,
		int totalLimit) {
	const auto bridge = Ayu::Ffi::RustBridge::Global();
	if (!bridge || totalLimit <= 0) {
		return {};
	}
	const auto raw = bridge->queryEditedRecords(
		std::int64_t(userId),
		std::int64_t(dialogId),
		std::int32_t(messageId),
		std::int64_t(minId),
		std::int64_t(maxId),
		std::size_t(totalLimit));
	auto result = std::vector<EditedMessage>();
	result.reserve(raw.size());
	for (const auto &row : raw) {
		auto msg = EditedMessage();
		if (DeserializeMessage(row.fakeId, row.payload, msg)) {
			result.push_back(std::move(msg));
		}
	}
	return result;
}

bool hasRevisions(ID userId, ID dialogId, ID messageId) {
	const auto bridge = Ayu::Ffi::RustBridge::Global();
	return bridge
		? bridge->hasEditedRecords(
			std::int64_t(userId),
			std::int64_t(dialogId),
			std::int32_t(messageId))
		: false;
}

void addDeletedMessage(const DeletedMessage &message) {
	if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
		bridge->insertDeletedRecord(
			std::int64_t(message.userId),
			std::int64_t(message.dialogId),
			std::int64_t(message.topicId),
			std::int32_t(message.messageId),
			QString::fromStdString(message.text),
			SerializeMessage(message));
	}
}

std::vector<DeletedMessage> getDeletedMessages(
		ID userId,
		ID dialogId,
		ID topicId,
		ID minId,
		ID maxId,
		int totalLimit,
		const std::string &searchQuery) {
	const auto bridge = Ayu::Ffi::RustBridge::Global();
	if (!bridge || totalLimit <= 0) {
		return {};
	}
	const auto raw = bridge->queryDeletedRecords(
		std::int64_t(userId),
		std::int64_t(dialogId),
		std::int64_t(topicId),
		std::int64_t(minId),
		std::int64_t(maxId),
		std::size_t(totalLimit),
		QString::fromStdString(searchQuery));
	auto result = std::vector<DeletedMessage>();
	result.reserve(raw.size());
	for (const auto &row : raw) {
		auto msg = DeletedMessage();
		if (DeserializeMessage(row.fakeId, row.payload, msg)) {
			result.push_back(std::move(msg));
		}
	}
	return result;
}

bool hasDeletedMessages(ID userId, ID dialogId, ID topicId) {
	const auto bridge = Ayu::Ffi::RustBridge::Global();
	return bridge
		? bridge->hasDeletedRecords(
			std::int64_t(userId),
			std::int64_t(dialogId),
			std::int64_t(topicId))
		: false;
}

void removeDeletedMessage(ID userId, ID dialogId, ID messageId) {
	if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
		bridge->removeDeletedRecord(
			std::int64_t(userId),
			std::int64_t(dialogId),
			std::int32_t(messageId));
	}
}

void clearDeletedMessages(ID userId, ID dialogId, ID topicId) {
	if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
		bridge->clearDeletedRecords(
			std::int64_t(userId),
			std::int64_t(dialogId),
			std::int64_t(topicId));
	}
}

std::vector<RegexFilter> getAllRegexFilters() {
	const auto bridge = Ayu::Ffi::RustBridge::Global();
	if (!bridge) {
		return {};
	}
	const auto items = bridge->listRegexFilters();
	auto out = std::vector<RegexFilter>();
	out.reserve(items.size());
	for (const auto &item : items) {
		out.push_back(FromBridgeFilter(item));
	}
	return out;
}

std::vector<RegexFilterGlobalExclusion> getAllFiltersExclusions() {
	const auto bridge = Ayu::Ffi::RustBridge::Global();
	if (!bridge) {
		return {};
	}
	const auto items = bridge->listRegexExclusions();
	auto out = std::vector<RegexFilterGlobalExclusion>();
	out.reserve(items.size());
	auto idx = ID(1);
	for (const auto &item : items) {
		auto ex = RegexFilterGlobalExclusion();
		ex.fakeId = idx++;
		ex.dialogId = ID(item.dialogId);
		ex.filterId = FromQByteArray(item.filterId);
		out.push_back(std::move(ex));
	}
	return out;
}

std::vector<RegexFilter> getExcludedByDialogId(ID dialogId) {
	const auto exclusions = getAllFiltersExclusions();
	const auto filters = getAllRegexFilters();
	auto out = std::vector<RegexFilter>();
	for (const auto &filter : filters) {
		const auto excluded = std::any_of(
			exclusions.begin(),
			exclusions.end(),
			[&](const RegexFilterGlobalExclusion &ex) {
				return (ex.dialogId == dialogId) && (ex.filterId == filter.id);
			});
		if (excluded) {
			out.push_back(filter);
		}
	}
	return out;
}

int getCount() {
	return int(getAllRegexFilters().size());
}

RegexFilter getById(std::vector<char> id) {
	for (const auto &filter : getAllRegexFilters()) {
		if (filter.id == id) {
			return filter;
		}
	}
	return {};
}

std::vector<RegexFilter> getShared() {
	auto out = std::vector<RegexFilter>();
	for (const auto &filter : getAllRegexFilters()) {
		if (!filter.dialogId.has_value()) {
			out.push_back(filter);
		}
	}
	return out;
}

std::vector<RegexFilter> getByDialogId(ID dialogId) {
	auto out = std::vector<RegexFilter>();
	for (const auto &filter : getAllRegexFilters()) {
		if (filter.dialogId && *filter.dialogId == dialogId) {
			out.push_back(filter);
		}
	}
	return out;
}

void addRegexFilter(const RegexFilter &filter) {
	if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
		bridge->upsertRegexFilter(ToBridgeFilter(filter));
	}
}

void addRegexExclusion(const RegexFilterGlobalExclusion &exclusion) {
	if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
		bridge->addRegexExclusion(
			std::int64_t(exclusion.dialogId),
			ToQByteArray(exclusion.filterId));
	}
}

void updateRegexFilter(const RegexFilter &filter) {
	if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
		bridge->upsertRegexFilter(ToBridgeFilter(filter));
	}
}

void deleteFilter(const std::vector<char> &id) {
	if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
		bridge->deleteRegexFilter(ToQByteArray(id));
	}
}

void deleteExclusionsByFilterId(const std::vector<char> &id) {
	if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
		bridge->deleteExclusionsByFilterId(ToQByteArray(id));
	}
}

void deleteExclusion(ID dialogId, std::vector<char> filterId) {
	if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
		bridge->removeRegexExclusion(
			std::int64_t(dialogId),
			ToQByteArray(filterId));
	}
}

void deleteAllFilters() {
	if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
		bridge->clearRegexFilters();
	}
}

void deleteAllExclusions() {
	if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
		bridge->clearRegexExclusions();
	}
}

bool hasFilters() {
	return !getAllRegexFilters().empty();
}

bool hasPerDialogFilters() {
	for (const auto &filter : getAllRegexFilters()) {
		if (filter.dialogId.has_value()) {
			return true;
		}
	}
	return !getAllFiltersExclusions().empty();
}

} // namespace AyuDatabase
