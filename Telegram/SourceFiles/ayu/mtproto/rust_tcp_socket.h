#pragma once

#include "mtproto/details/mtproto_abstract_socket.h"

#include <cstdint>

namespace MTP::details {

class RustTcpSocket final : public AbstractSocket {
public:
	explicit RustTcpSocket(not_null<QThread*> thread);
	~RustTcpSocket() override;

	void connectToHost(const QString &address, int port) override;
	[[nodiscard]] bool isGoodStartNonce(bytes::const_span nonce) override;
	void timedOut() override;
	[[nodiscard]] bool isConnected() override;
	[[nodiscard]] bool hasBytesAvailable() override;
	[[nodiscard]] int64 read(bytes::span buffer) override;
	void write(
		bytes::const_span prefix,
		bytes::const_span buffer) override;

	int32 debugState() override;
	[[nodiscard]] QString debugPostfix() const override;

	void handleRustEvent(std::uint8_t eventKind);

private:
	std::uint64_t _socketId = 0;

};

} // namespace MTP::details
