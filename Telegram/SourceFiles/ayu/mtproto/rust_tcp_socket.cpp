#include "ayu/mtproto/rust_tcp_socket.h"

#include "ayu/ffi/ayu_rust_bridge.h"
#include "base/invoke_queued.h"

#include <QtCore/QPointer>
#include <QtNetwork/QAbstractSocket>
#include <cstring>
#include <mutex>
#include <unordered_map>

namespace MTP::details {
namespace {

constexpr auto kEventConnected = std::uint8_t(1);
constexpr auto kEventReadyRead = std::uint8_t(2);
constexpr auto kEventDisconnected = std::uint8_t(3);
constexpr auto kEventError = std::uint8_t(4);

struct SocketEntry {
	QPointer<QObject> guard;
	RustTcpSocket *socket = nullptr;
};

std::mutex gRegistryMutex;
std::unordered_map<std::uint64_t, SocketEntry> gSockets;

void OnRustTcpEvent(std::uint64_t socketId, std::uint8_t eventKind) {
	auto entry = SocketEntry();
	{
		std::lock_guard lock(gRegistryMutex);
		const auto it = gSockets.find(socketId);
		if (it != gSockets.end()) {
			entry = it->second;
		}
	}
	if (auto *obj = entry.guard.data()) {
		InvokeQueued(obj, [guard = entry.guard, sock = entry.socket, eventKind] {
			if (guard && sock) {
				sock->handleRustEvent(eventKind);
			}
		});
	}
}

void EnsureHookRegistered() {
	static std::once_flag once;
	std::call_once(once, [] {
		Ayu::Ffi::RustBridge::SetTcpNotifyHook(&OnRustTcpEvent);
	});
}

} // namespace

RustTcpSocket::RustTcpSocket(not_null<QThread*> thread)
: AbstractSocket(thread) {
	EnsureHookRegistered();
}

RustTcpSocket::~RustTcpSocket() {
	if (_socketId != 0) {
		{
			std::lock_guard lock(gRegistryMutex);
			gSockets.erase(_socketId);
		}
		if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
			bridge->tcpClose(_socketId);
		}
		_socketId = 0;
	}
}

void RustTcpSocket::connectToHost(const QString &address, int port) {
	if (_socketId != 0) {
		{
			std::lock_guard lock(gRegistryMutex);
			gSockets.erase(_socketId);
		}
		if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
			bridge->tcpClose(_socketId);
		}
		_socketId = 0;
	}
	const auto bridge = Ayu::Ffi::RustBridge::Global();
	if (!bridge) {
		_error.fire({});
		return;
	}
	{
		std::lock_guard lock(gRegistryMutex);
		_socketId = bridge->tcpConnect(address, std::uint16_t(port));
		if (_socketId == 0) {
			_error.fire({});
			return;
		}
		gSockets[_socketId] = SocketEntry{
			.guard = QPointer<QObject>(this),
			.socket = this,
		};
	}
}

bool RustTcpSocket::isGoodStartNonce(bytes::const_span nonce) {
	Expects(nonce.size() >= 2 * sizeof(uint32));

	const auto bytes = nonce.data();
	const auto zero = *reinterpret_cast<const uchar*>(bytes);
	const auto first = *reinterpret_cast<const uint32*>(bytes);
	const auto second = *(reinterpret_cast<const uint32*>(bytes) + 1);
	const auto reserved01 = 0x000000EFU;
	const auto reserved11 = 0x44414548U;
	const auto reserved12 = 0x54534F50U;
	const auto reserved13 = 0x20544547U;
	const auto reserved14 = 0xEEEEEEEEU;
	const auto reserved15 = 0xDDDDDDDDU;
	const auto reserved16 = 0x02010316U;
	const auto reserved21 = 0x00000000U;
	return (zero != reserved01)
		&& (first != reserved11)
		&& (first != reserved12)
		&& (first != reserved13)
		&& (first != reserved14)
		&& (first != reserved15)
		&& (first != reserved16)
		&& (second != reserved21);
}

void RustTcpSocket::timedOut() {
	if (_socketId != 0) {
		if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
			bridge->tcpClose(_socketId);
		}
	}
}

bool RustTcpSocket::isConnected() {
	const auto bridge = Ayu::Ffi::RustBridge::Global();
	return (bridge && _socketId != 0)
		? bridge->tcpIsConnected(_socketId)
		: false;
}

bool RustTcpSocket::hasBytesAvailable() {
	const auto bridge = Ayu::Ffi::RustBridge::Global();
	return (bridge && _socketId != 0)
		? bridge->tcpHasBytes(_socketId)
		: false;
}

int64 RustTcpSocket::read(bytes::span buffer) {
	const auto bridge = Ayu::Ffi::RustBridge::Global();
	if (!bridge || _socketId == 0 || buffer.empty()) {
		return 0;
	}
	const auto data = bridge->tcpRead(_socketId, std::size_t(buffer.size()));
	if (data.isEmpty()) {
		return 0;
	}
	std::memcpy(buffer.data(), data.constData(), std::size_t(data.size()));
	return int64(data.size());
}

void RustTcpSocket::write(bytes::const_span prefix, bytes::const_span buffer) {
	Expects(!buffer.empty());

	const auto bridge = Ayu::Ffi::RustBridge::Global();
	if (!bridge || _socketId == 0) {
		return;
	}
	if (!prefix.empty()) {
		bridge->tcpWrite(_socketId, prefix.data(), std::size_t(prefix.size()));
	}
	bridge->tcpWrite(_socketId, buffer.data(), std::size_t(buffer.size()));
}

int32 RustTcpSocket::debugState() {
	return isConnected()
		? int32(QAbstractSocket::ConnectedState)
		: int32(QAbstractSocket::UnconnectedState);
}

QString RustTcpSocket::debugPostfix() const {
	return u"_rust"_q;
}

void RustTcpSocket::handleRustEvent(std::uint8_t eventKind) {
	switch (eventKind) {
	case kEventConnected:
		_connected.fire({});
		break;
	case kEventReadyRead:
		_readyRead.fire({});
		break;
	case kEventDisconnected:
		_disconnected.fire({});
		break;
	case kEventError:
		logError(QAbstractSocket::NetworkError, u"rust tokio tcp error"_q);
		_error.fire({});
		break;
	default:
		break;
	}
}

} // namespace MTP::details
