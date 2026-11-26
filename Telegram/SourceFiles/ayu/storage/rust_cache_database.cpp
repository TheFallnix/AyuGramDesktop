// Rust-backed Storage::Cache::Database implementation.
#include "storage/cache/storage_cache_database.h"

#include "ayu/ffi/ayu_rust_bridge.h"
#include "storage/storage_encryption.h"

#include <crl/crl_semaphore.h>
#include <rpl/variable.h>

namespace Storage {
namespace Cache {
namespace details {

class DatabaseObject {
public:
	using Settings = Cache::Database::Settings;

	DatabaseObject(
		crl::weak_on_queue<DatabaseObject> weak,
		const QString &path,
		const Settings &settings)
	: _ns(ComputeBasePath(path))
	, _settings(settings) {
	}

	void reconfigure(const Settings &settings) {
		_settings = settings;
	}

	void updateSettings(const SettingsUpdate &update) {
		_settings.totalSizeLimit = update.totalSizeLimit;
		_settings.totalTimeLimit = update.totalTimeLimit;
	}

	void open(EncryptionKey &&key, FnMut<void(Error)> &&done) {
		_opened = true;
		refreshStats();
		if (done) {
			done(Error::NoError());
		}
	}

	void close(FnMut<void()> &&done) {
		_opened = false;
		if (done) {
			done();
		}
	}

	void put(
			const Key &key,
			TaggedValue &&value,
			FnMut<void(Error)> &&done) {
		putImpl(key, std::move(value), false, std::move(done));
	}

	void putIfEmpty(
			const Key &key,
			TaggedValue &&value,
			FnMut<void(Error)> &&done) {
		putImpl(key, std::move(value), true, std::move(done));
	}

	void get(const Key &key, FnMut<void(TaggedValue&&)> &&done) {
		if (!done) {
			return;
		}
		if (!_opened || !key.valid()) {
			done(TaggedValue());
			return;
		}
		if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
			auto res = bridge->kvGet(_ns, key.high, key.low);
			if (res.found) {
				done(TaggedValue(std::move(res.data), res.tag));
				return;
			}
		}
		done(TaggedValue());
	}

	void remove(const Key &key, FnMut<void(Error)> &&done) {
		if (_opened && key.valid()) {
			if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
				bridge->kvRemove(_ns, key.high, key.low);
				refreshStats();
			}
		}
		if (done) {
			done(Error::NoError());
		}
	}

	void copyIfEmpty(
			const Key &from,
			const Key &to,
			FnMut<void(Error)> &&done) {
		copyOrMoveImpl(from, to, false, std::move(done));
	}

	void moveIfEmpty(
			const Key &from,
			const Key &to,
			FnMut<void(Error)> &&done) {
		copyOrMoveImpl(from, to, true, std::move(done));
	}

	void getWithSizes(
			const Key &key,
			std::vector<Key> &&keys,
			FnMut<void(QByteArray&&, std::vector<int>&&)> &&done) {
		if (!done) {
			return;
		}
		if (!_opened || !key.valid()) {
			done(QByteArray(), std::vector<int>());
			return;
		}
		const auto bridge = Ayu::Ffi::RustBridge::Global();
		if (!bridge) {
			done(QByteArray(), std::vector<int>());
			return;
		}
		auto res = bridge->kvGet(_ns, key.high, key.low);
		if (!res.found || res.data.isEmpty()) {
			done(QByteArray(), std::vector<int>());
			return;
		}
		auto hi = std::vector<std::uint64_t>();
		auto lo = std::vector<std::uint64_t>();
		hi.reserve(keys.size());
		lo.reserve(keys.size());
		for (const auto &k : keys) {
			hi.push_back(k.high);
			lo.push_back(k.low);
		}
		auto sizes = bridge->kvGetSizes(_ns, hi, lo);
		for (auto &sz : sizes) {
			if (sz < 0) {
				sz = 0;
			}
		}
		done(std::move(res.data), std::move(sizes));
	}

	[[nodiscard]] rpl::producer<Stats> stats() const {
		return _stats.value();
	}

	void clear(FnMut<void(Error)> &&done) {
		if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
			bridge->kvClear(_ns);
			refreshStats();
		}
		if (done) {
			done(Error::NoError());
		}
	}

	void clearByTag(uint8 tag, FnMut<void(Error)> &&done) {
		if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
			bridge->kvClearByTag(_ns, tag);
			refreshStats();
		}
		if (done) {
			done(Error::NoError());
		}
	}

	void waitForCleaner(FnMut<void()> &&done) {
		if (done) {
			done();
		}
	}

private:
	void putImpl(
			const Key &key,
			TaggedValue &&value,
			bool onlyIfEmpty,
			FnMut<void(Error)> &&done) {
		if (_opened && key.valid() && !value.bytes.isEmpty()) {
			if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
				bridge->kvPut(
					_ns,
					key.high,
					key.low,
					value.tag,
					value.bytes,
					onlyIfEmpty,
					std::int64_t(_settings.totalSizeLimit));
				refreshStats();
			}
		}
		if (done) {
			done(Error::NoError());
		}
	}

	void copyOrMoveImpl(
			const Key &from,
			const Key &to,
			bool isMove,
			FnMut<void(Error)> &&done) {
		if (_opened && from.valid() && to.valid()) {
			if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
				bridge->kvCopyOrMoveIfEmpty(
					_ns,
					from.high,
					from.low,
					to.high,
					to.low,
					isMove);
				refreshStats();
			}
		}
		if (done) {
			done(Error::NoError());
		}
	}

	void refreshStats() {
		auto next = Stats();
		if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
			for (const auto &row : bridge->kvStats(_ns)) {
				const auto summary = TaggedSummary{
					.count = size_type(row.count),
					.totalSize = int64(row.totalSize),
				};
				next.tagged[row.tag] = summary;
				next.full.count += summary.count;
				next.full.totalSize += summary.totalSize;
			}
		}
		_stats = std::move(next);
	}

	QString _ns;
	Settings _settings;
	bool _opened = false;
	rpl::variable<Stats> _stats;

};

} // namespace details

Database::Database(const QString &path, const Settings &settings)
: _wrapped(path, settings) {
}

void Database::reconfigure(const Settings &settings) {
	_wrapped.with([settings](Implementation &unwrapped) mutable {
		unwrapped.reconfigure(settings);
	});
}

void Database::updateSettings(const SettingsUpdate &update) {
	_wrapped.with([update](Implementation &unwrapped) mutable {
		unwrapped.updateSettings(update);
	});
}

void Database::open(EncryptionKey &&key, FnMut<void(Error)> &&done) {
	_wrapped.with([
		key = std::move(key),
		done = std::move(done)
	](Implementation &unwrapped) mutable {
		unwrapped.open(std::move(key), std::move(done));
	});
}

void Database::close(FnMut<void()> &&done) {
	_wrapped.with([
		done = std::move(done)
	](Implementation &unwrapped) mutable {
		unwrapped.close(std::move(done));
	});
}

void Database::waitForCleaner(FnMut<void()> &&done) {
	_wrapped.with([
		done = std::move(done)
	](Implementation &unwrapped) mutable {
		unwrapped.waitForCleaner(std::move(done));
	});
}

void Database::put(
		const Key &key,
		QByteArray &&value,
		FnMut<void(Error)> &&done) {
	return put(key, TaggedValue(std::move(value), 0), std::move(done));
}

void Database::get(const Key &key, FnMut<void(QByteArray&&)> &&done) {
	if (done) {
		auto untag = [done = std::move(done)](TaggedValue &&value) mutable {
			done(std::move(value.bytes));
		};
		getWithTag(key, std::move(untag));
	} else {
		getWithTag(key, nullptr);
	}
}

void Database::remove(const Key &key, FnMut<void(Error)> &&done) {
	_wrapped.with([
		key,
		done = std::move(done)
	](Implementation &unwrapped) mutable {
		unwrapped.remove(key, std::move(done));
	});
}

void Database::putIfEmpty(
		const Key &key,
		QByteArray &&value,
		FnMut<void(Error)> &&done) {
	return putIfEmpty(
		key,
		TaggedValue(std::move(value), 0),
		std::move(done));
}

void Database::copyIfEmpty(
		const Key &from,
		const Key &to,
		FnMut<void(Error)> &&done) {
	_wrapped.with([
		from,
		to,
		done = std::move(done)
	](Implementation &unwrapped) mutable {
		unwrapped.copyIfEmpty(from, to, std::move(done));
	});
}

void Database::moveIfEmpty(
		const Key &from,
		const Key &to,
		FnMut<void(Error)> &&done) {
	_wrapped.with([
		from,
		to,
		done = std::move(done)
	](Implementation &unwrapped) mutable {
		unwrapped.moveIfEmpty(from, to, std::move(done));
	});
}

void Database::put(
		const Key &key,
		TaggedValue &&value,
		FnMut<void(Error)> &&done) {
	_wrapped.with([
		key,
		value = std::move(value),
		done = std::move(done)
	](Implementation &unwrapped) mutable {
		unwrapped.put(key, std::move(value), std::move(done));
	});
}

void Database::putIfEmpty(
		const Key &key,
		TaggedValue &&value,
		FnMut<void(Error)> &&done) {
	_wrapped.with([
		key,
		value = std::move(value),
		done = std::move(done)
	](Implementation &unwrapped) mutable {
		unwrapped.putIfEmpty(key, std::move(value), std::move(done));
	});
}

void Database::getWithTag(
		const Key &key,
		FnMut<void(TaggedValue&&)> &&done) {
	_wrapped.with([
		key,
		done = std::move(done)
	](Implementation &unwrapped) mutable {
		unwrapped.get(key, std::move(done));
	});
}

void Database::getWithSizes(
		const Key &key,
		std::vector<Key> &&keys,
		FnMut<void(QByteArray&&, std::vector<int>&&)> &&done) {
	_wrapped.with([
		key,
		keys = std::move(keys),
		done = std::move(done)
	](Implementation &unwrapped) mutable {
		unwrapped.getWithSizes(key, std::move(keys), std::move(done));
	});
}

auto Database::statsOnMain() const -> rpl::producer<Stats> {
	return _wrapped.producer_on_main([](const Implementation &unwrapped) {
		return unwrapped.stats();
	});
}

void Database::clear(FnMut<void(Error)> &&done) {
	_wrapped.with([
		done = std::move(done)
	](Implementation &unwrapped) mutable {
		unwrapped.clear(std::move(done));
	});
}

void Database::clearByTag(uint8 tag, FnMut<void(Error)> &&done) {
	_wrapped.with([
		tag,
		done = std::move(done)
	](Implementation &unwrapped) mutable {
		unwrapped.clearByTag(tag, std::move(done));
	});
}

void Database::sync() {
	auto semaphore = crl::semaphore();
	_wrapped.with([&](Implementation &) {
		semaphore.release();
	});
	semaphore.acquire();
}

Database::~Database() = default;

} // namespace Cache
} // namespace Storage
