// This is the source code of AyuGram for Desktop.
//
// We do not and cannot prevent the use of our code,
// but be respectful and credit the original author.
//
// Copyright @Radolyn, 2026
#include "ayu/ui/settings/filters/edit_filter.h"

#include "lang_auto.h"
#include "ayu/ayu_settings.h"
#include "ayu/data/ayu_database.h"
#include "ayu/features/filters/filters_cache_controller.h"
#include "ayu/ffi/ayu_rust_bridge.h"
#include "ayu/ui/toasts.h"
#include "base/event_filter.h"
#include "base/platform/base_platform_info.h"
#include "boxes/delete_messages_box.h"
#include "core/mime_type.h"
#include "lang/lang_text_entity.h"
#include "media/audio/media_audio.h"
#include "media/view/media_view_pip.h"
#include "styles/style_boxes.h"
#include "styles/style_layers.h"
#include "styles/style_settings.h"
#include "styles/style_window.h"
#include "ui/ui_utility.h"
#include "ui/boxes/confirm_box.h"
#include "ui/text/text.h"
#include "ui/toast/toast.h"
#include "ui/widgets/checkbox.h"
#include "ui/widgets/labels.h"
#include "ui/widgets/fields/input_field.h"
#include "ui/wrap/slide_wrap.h"
#include "ui/wrap/vertical_layout.h"

namespace Settings {

std::vector<char> generate_uuid_bytes() {
	// stolen somewhere from Internet
	std::random_device rd;
	std::mt19937 gen(rd());
	std::uniform_int_distribution<uint32_t> dist;

	std::vector<uint8_t> bytes(16);
	for (int i = 0; i < 16; i += 4) {
		uint32_t random_chunk = dist(gen);
		bytes[i] = random_chunk & 0xFF;
		bytes[i + 1] = (random_chunk >> 8) & 0xFF;
		bytes[i + 2] = (random_chunk >> 16) & 0xFF;
		bytes[i + 3] = (random_chunk >> 24) & 0xFF;
	}
	bytes[6] = (bytes[6] & 0x0F) | 0x40;
	bytes[8] = (bytes[8] & 0x3F) | 0x80;

	return std::vector<char>(bytes.begin(), bytes.end());
}

bool validateRegex(const QString &pattern, QString &errorMsg) {
	if (const auto bridge = Ayu::Ffi::RustBridge::Global()) {
		return bridge->validateRegexPattern(pattern, errorMsg);
	}
	return true;
}

not_null<Ui::SlideWrap<Ui::FlatLabel>*> AddError(
	not_null<Ui::VerticalLayout*> content,
	Ui::InputField *input) {

	QString errorText;
	validateRegex(u"("_q, errorText);

	const auto error = content->add(
		object_ptr<Ui::SlideWrap<Ui::FlatLabel>>(
			content,
			object_ptr<Ui::FlatLabel>(
				content,
				errorText + errorText,
				st::settingLocalPasscodeError), st::settingsCheckboxPadding));
	error->hide(anim::type::instant);
	if (input) {
		input->changes() | rpl::on_next(
			[=]
			{
				error->hide(anim::type::normal);
			},
			input->lifetime());
	}
	return error;
};

void RegexEditBuilder(
	not_null<Ui::GenericBox*> box,
	RegexFilter *filter,
	const Fn<void(RegexFilter)> &onDone,
	std::optional<long long> dialogId,
	bool showToast
) {
	RegexFilter data;

	if (filter) {
		box->setTitle(showToast ? tr::ayu_RegexFiltersAdd() : tr::ayu_RegexFiltersEdit());
		data = *filter;
	} else {
		box->setTitle(tr::ayu_RegexFiltersAdd());
		data.enabled = true;
		data.caseInsensitive = true;
		data.reversed = false;
	}

	const auto regexValue = box->addRow(
		object_ptr<Ui::InputField>(
			box->verticalLayout(),
			st::windowFilterNameInput,
			Ui::InputField::Mode::MultiLine,
			tr::ayu_RegexFiltersPlaceholder()),
		st::markdownLinkFieldPadding);
	const auto errorText = AddError(box->verticalLayout(), regexValue);
	const auto enabled = box->addRow(
		object_ptr<Ui::Checkbox>(
			box,
			tr::ayu_EnableExpression(tr::now),
			data.enabled,
			st::defaultBoxCheckbox),
		st::settingsCheckboxPadding);
	const auto caseInsensitive = box->addRow(
		object_ptr<Ui::Checkbox>(
			box,
			tr::ayu_CaseInsensitiveExpression(tr::now),
			data.caseInsensitive,
			st::defaultBoxCheckbox),
		st::settingsCheckboxPadding);
	const auto reversed = box->addRow(
		object_ptr<Ui::Checkbox>(
			box,
			tr::ayu_ReversedExpression(tr::now),
			data.reversed,
			st::defaultBoxCheckbox),
		st::settingsCheckboxPadding);

	regexValue->setText(QString::fromStdString(data.text));

	auto saveAndClose = [=, id = data.id]
	{
		const auto text = regexValue->getTextWithTags().text;
		if (text.isEmpty()) {
			return;
		}

		QString error;
		if (!validateRegex(text, error)) {
			errorText->entity()->setText(error);
			errorText->show(anim::type::normal);
			return;
		}

		RegexFilter newFilter;
		newFilter.text = regexValue->getTextWithTags().text.toStdString();
		newFilter.enabled = enabled->checked();
		newFilter.caseInsensitive = caseInsensitive->checked();
		newFilter.reversed = reversed->checked();

		if (!showToast && dialogId.has_value()) {
			newFilter.dialogId = dialogId;
		}

		if (!id.empty()) {
			newFilter.id = id;
		} else {
			newFilter.id = generate_uuid_bytes();
		}

		box->closeBox();

		crl::async([=]
		{
			AyuDatabase::addRegexFilter(newFilter);
			FiltersCacheController::rebuildCache();

			crl::on_main([=]
			{
				if (onDone) {
					onDone(newFilter);
				}
				FiltersCacheController::fireUpdate();

				if (showToast) {
					auto config = Ui::Toast::Config{
						.text = tr::ayu_RegexFilterBulletinText(
							tr::now,
							tr::rich),
						.adaptive = true,
					};
					if (dialogId.has_value()) {
						Ayu::Ui::ShowToastWithAction(
							std::move(config),
							tr::ayu_RegexFilterBulletinAction(tr::now),
							[=]() mutable {
								newFilter.dialogId = dialogId;

								AyuDatabase::updateRegexFilter(newFilter);
								FiltersCacheController::rebuildCache();
								FiltersCacheController::fireUpdate();
							});
					} else {
						Ui::Toast::Show(std::move(config));
					}
				}
			});
		});
	};

	regexValue->submits() | rpl::on_next(saveAndClose, regexValue->lifetime());
	box->addButton(tr::lng_settings_save(), saveAndClose);
	box->addButton(tr::lng_cancel(),
				   [=]
				   {
					   box->closeBox();
				   });
	box->setFocusCallback([=] {
		regexValue->setFocusFast();
	});

	errorText->entity()->resizeToWidth(box->width());
	errorText->resizeToWidth(box->width());
}

object_ptr<Ui::GenericBox> RegexEditBox(RegexFilter *filter,
										const Fn<void(RegexFilter)> &onDone,
										std::optional<long long> dialogId,
										bool showToast) {
	return Box(RegexEditBuilder, filter, onDone, dialogId, showToast);
}

} // namespace Settings
