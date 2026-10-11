// Dev probe: lists the UI Automation properties of every Edit and Document
// in windows whose title contains the first argument. Only metadata and the
// Document Value (a browser reports the page URL there) are printed; it is
// meant for local test pages, never for real user windows.
// Run: cargo run --example uia_probe -- "GLIM-PROBE"

#[cfg(windows)]
fn main() -> windows::core::Result<()> {
    use windows::core::BSTR;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
    use windows::Win32::System::Variant::VARIANT;
    use windows::Win32::UI::Accessibility::*;

    let needle = std::env::args().nth(1).unwrap_or_else(|| "GLIM-PROBE".into());
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let uia: IUIAutomation = CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)?;
        let root = uia.GetRootElement()?;
        let windows = root.FindAll(TreeScope_Children, &uia.CreateTrueCondition()?)?;
        let edit = uia.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_EditControlTypeId.0))?;
        let doc = uia.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_DocumentControlTypeId.0))?;
        let either = uia.CreateOrCondition(&edit, &doc)?;
        let props: [(&str, UIA_PROPERTY_ID); 11] = [
            ("Name", UIA_NamePropertyId),
            ("AutomationId", UIA_AutomationIdPropertyId),
            ("HelpText", UIA_HelpTextPropertyId),
            ("FullDescription", UIA_FullDescriptionPropertyId),
            ("AriaRole", UIA_AriaRolePropertyId),
            ("AriaProperties", UIA_AriaPropertiesPropertyId),
            ("LocalizedControlType", UIA_LocalizedControlTypePropertyId),
            ("ClassName", UIA_ClassNamePropertyId),
            ("ItemStatus", UIA_ItemStatusPropertyId),
            ("LegacyDescription", UIA_LegacyIAccessibleDescriptionPropertyId),
            ("LegacyHelp", UIA_LegacyIAccessibleHelpPropertyId),
        ];
        for i in 0..windows.Length()? {
            let w = windows.GetElement(i)?;
            let title: BSTR = w.CurrentName().unwrap_or_default();
            if !title.to_string().contains(&needle) {
                continue;
            }
            println!("== window: {title}");
            let found = w.FindAll(TreeScope_Descendants, &either)?;
            for j in 0..found.Length()? {
                let e = found.GetElement(j)?;
                let ct = e.CurrentControlType().map(|c| c.0).unwrap_or(0);
                println!("-- {}", if ct == UIA_EditControlTypeId.0 { "Edit" } else { "Document" });
                for (label, id) in props {
                    if let Ok(v) = e.GetCurrentPropertyValue(id) {
                        let inner = &v.Anonymous.Anonymous;
                        let s = if inner.vt == windows::Win32::System::Variant::VT_BSTR {
                            inner.Anonymous.bstrVal.to_string()
                        } else {
                            String::new()
                        };
                        if !s.is_empty() {
                            println!("   {label}: {s}");
                        }
                    }
                }
                if let Ok(by) = e.CurrentLabeledBy() {
                    println!("   LabeledBy.Name: {}", by.CurrentName().unwrap_or_default());
                }
                if ct == UIA_DocumentControlTypeId.0 {
                    if let Ok(v) = e.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) {
                        println!("   Value: {}", v.CurrentValue().unwrap_or_default());
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn main() {}
