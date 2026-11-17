// This is the source code of AyuGram for Desktop.
//
// We do not and cannot prevent the use of our code,
// but be respectful and credit the original author.
//
// Copyright @Radolyn, 2026
#include "ayu/ayu_infra.h"

#include "ayu/ayu_lang.h"
#include "ayu/ayu_settings.h"
#include "ayu/ayu_ui_settings.h"
#include "ayu/ayu_worker.h"
#include "ayu/data/ayu_database.h"
#include "ayu/ffi/ayu_rust_bridge.h"
#include "ayu/ui/ayu_logo.h"
#include "core/application.h"
#include "features/translator/ayu_translator.h"
#include "lang/lang_instance.h"
#include "main/main_account.h"
#include "main/main_domain.h"
#include "main/main_session.h"
#include "mtproto/mtp_instance.h"
#include "ui/chat/chat_style_radius.h"
#include "utils/rc_manager.h"

#ifdef Q_OS_WIN
#include "ayu/utils/windows_utils.h"
#endif

namespace AyuInfra {

void initLang() {
	QString id = Lang::GetInstance().id();
	QString baseId = Lang::GetInstance().baseId();
	if (id.isEmpty()) {
		LOG(("Language is not loaded"));
		return;
	}
	AyuLanguage::init();
	AyuLanguage::currentInstance()->fetchLanguage(id, baseId);
}

void initUiSettings() {
	const auto &settings = AyuSettings::getInstance();

	AyuUiSettings::setMonoFont(settings.monoFont());
	AyuUiSettings::setWideMultiplier(settings.wideMultiplier());
	AyuUiSettings::setMaterialSwitches(settings.materialSwitches());
	AyuUiSettings::setAvatarCorners(settings.avatarCorners());
	Ui::SetAppliedBubbleRadius(settings.messageBubbleRadius());
}

void initDatabase() {
	AyuDatabase::initialize();
	Ayu::Ffi::RustBridge::InitializeGlobal(u"./tdata/ayu_rust.db"_q, 4);
	if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
		const auto &domain = Core::App().domain();
		for (const auto &[index, account] : domain.accounts()) {
			if (!account) {
				continue;
			}
			const auto id = std::uint64_t(index);
			const auto session = account->maybeSession();
			const auto userId = session ? session->userId().bare : uint64(0);
			const auto isTest = account->mtp().isTestMode();
			bridge->registerAccount(id, userId, isTest);
			const auto serialized = account->serializeMtpAuthorization();
			if (!serialized.isEmpty()) {
				bridge->importMtpAuthorization(id, serialized);
			}
			if (session) {
				bridge->completeAuthorization(
					id,
					userId,
					account->mtp().mainDcId(),
					session->premium());
			}
		}
		if (domain.started()) {
			for (const auto &[index, account] : domain.accounts()) {
				if (account.get() == &domain.active()) {
					bridge->activateAccount(std::uint64_t(index));
					break;
				}
			}
		}
	}
}

void initWorker() {
	AyuWorker::initialize();
}

void initRCManager() {
	RCManager::getInstance().start();
}

void initTranslator() {
	Ayu::Translator::TranslateManager::init();
}

void initIcon() {
#ifdef Q_OS_WIN
	AyuAssets::loadAppIco();
	reloadAppIconFromTaskBar();
#endif
}

void init() {
	initLang();
	initDatabase();
	initUiSettings();
	initIcon();
	initWorker();
	initRCManager();
	initTranslator();
}

}
