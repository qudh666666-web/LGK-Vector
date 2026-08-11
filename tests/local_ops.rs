use std::fs;

use lgk_vector::daemon::commands::CommandDispatcher;
use lgk_vector::ops;
use lgk_vector::project::SessionConfig;
use serde_json::json;
use tempfile::tempdir;

fn fixture() -> (tempfile::TempDir, SessionConfig) {
    let root = tempdir().expect("tempdir");
    let project = root.path().join("Cfg");
    let tool = root.path().join("SIP");
    fs::create_dir_all(project.join("Config/ECUC")).expect("project dirs");
    fs::create_dir_all(tool.join("BSWMD/Com")).expect("tool dirs");
    fs::create_dir_all(tool.join("DaVinciConfigurator/Core")).expect("DaVinci dirs");
    fs::write(tool.join("DaVinciConfigurator/Core/DVCfgCmd.exe"), b"")
        .expect("DaVinci command placeholder");
    fs::write(
        project.join("Test.dpa"),
        r#"<?xml version="1.0"?>
<ProjectAssistant>
  <EcucSplitter>
    <Splitter File=".\Config\ECUC\Test_Com_ecuc.arxml">
      <Module Name="Com"/>
    </Splitter>
  </EcucSplitter>
</ProjectAssistant>"#,
    )
    .expect("dpa");
    fs::write(
        project.join("Config/ECUC/Test_Com_ecuc.arxml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<AUTOSAR>
  <ECUC-MODULE-CONFIGURATION-VALUES>
    <SHORT-NAME>Com</SHORT-NAME>
    <DEFINITION-REF DEST="ECUC-MODULE-DEF">/MICROSAR/Com</DEFINITION-REF>
    <CONTAINERS>
      <ECUC-CONTAINER-VALUE>
        <SHORT-NAME>ComConfig</SHORT-NAME>
        <DEFINITION-REF DEST="ECUC-PARAM-CONF-CONTAINER-DEF">/MICROSAR/Com/ComConfig</DEFINITION-REF>
        <SUB-CONTAINERS>
          <ECUC-CONTAINER-VALUE>
            <SHORT-NAME>SignalA</SHORT-NAME>
            <DEFINITION-REF DEST="ECUC-PARAM-CONF-CONTAINER-DEF">/MICROSAR/Com/ComConfig/ComSignal</DEFINITION-REF>
            <PARAMETER-VALUES>
              <ECUC-NUMERICAL-PARAM-VALUE>
                <DEFINITION-REF DEST="ECUC-INTEGER-PARAM-DEF">/MICROSAR/Com/ComConfig/ComSignal/ComBitPosition</DEFINITION-REF>
                <VALUE>8</VALUE>
              </ECUC-NUMERICAL-PARAM-VALUE>
            </PARAMETER-VALUES>
          </ECUC-CONTAINER-VALUE>
        </SUB-CONTAINERS>
      </ECUC-CONTAINER-VALUE>
    </CONTAINERS>
  </ECUC-MODULE-CONFIGURATION-VALUES>
</AUTOSAR>
"#,
    )
    .expect("config");
    fs::write(
        tool.join("BSWMD/Com/Com_bswmd.arxml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<AUTOSAR>
  <AR-PACKAGES>
    <AR-PACKAGE>
      <SHORT-NAME>MICROSAR</SHORT-NAME>
      <ELEMENTS>
        <ECUC-MODULE-DEF>
          <SHORT-NAME>Com</SHORT-NAME>
          <CONTAINERS>
            <ECUC-PARAM-CONF-CONTAINER-DEF>
              <SHORT-NAME>ComConfig</SHORT-NAME>
              <SUB-CONTAINERS>
                <ECUC-PARAM-CONF-CONTAINER-DEF>
                  <SHORT-NAME>ComSignal</SHORT-NAME>
                  <PARAMETERS>
                    <ECUC-INTEGER-PARAM-DEF>
                      <SHORT-NAME>ComBitPosition</SHORT-NAME>
                      <DESC><L-2 L="EN">Starting position in the I-PDU.</L-2></DESC>
                      <DEFAULT-VALUE>0</DEFAULT-VALUE>
                      <MIN>0</MIN>
                      <MAX>65535</MAX>
                    </ECUC-INTEGER-PARAM-DEF>
                  </PARAMETERS>
                </ECUC-PARAM-CONF-CONTAINER-DEF>
                <ECUC-CHOICE-CONTAINER-DEF>
                  <SHORT-NAME>ComGwDestination</SHORT-NAME>
                  <LOWER-MULTIPLICITY>0</LOWER-MULTIPLICITY>
                  <CHOICES>
                    <ECUC-PARAM-CONF-CONTAINER-DEF>
                      <SHORT-NAME>ComGwSignal</SHORT-NAME>
                      <PARAMETERS>
                        <ECUC-INTEGER-PARAM-DEF>
                          <SHORT-NAME>ComGwSignalBitPosition</SHORT-NAME>
                          <MAX>4095</MAX>
                        </ECUC-INTEGER-PARAM-DEF>
                      </PARAMETERS>
                    </ECUC-PARAM-CONF-CONTAINER-DEF>
                  </CHOICES>
                </ECUC-CHOICE-CONTAINER-DEF>
              </SUB-CONTAINERS>
            </ECUC-PARAM-CONF-CONTAINER-DEF>
          </CONTAINERS>
        </ECUC-MODULE-DEF>
      </ELEMENTS>
    </AR-PACKAGE>
  </AR-PACKAGES>
</AUTOSAR>"#,
    )
    .expect("template");
    fs::write(
        project.join("lgk-vector.json"),
        format!(
            "{{\"project_path\":{},\"tool_path\":{}}}",
            serde_json::to_string(&project).expect("project JSON"),
            serde_json::to_string(&tool).expect("tool JSON")
        ),
    )
    .expect("config file");
    let config = SessionConfig::load(&project).expect("load session");
    (root, config)
}

#[test]
fn accepts_utf8_bom_in_project_configuration() {
    let (root, config) = fixture();
    let config_path = config.project_path.join("lgk-vector.json");
    let raw = fs::read_to_string(&config_path).expect("read config");
    fs::write(&config_path, format!("\u{feff}{raw}")).expect("write BOM config");
    let reloaded = SessionConfig::load(&config.project_path).expect("load BOM config");
    assert_eq!(reloaded.project_path, config.project_path);
    drop(root);
}

#[test]
fn finds_module_and_definition() {
    let (_root, config) = fixture();
    let module =
        ops::find_module::execute(&config, &json!({"module": "Com"})).expect("find module");
    assert_eq!(module["module"], "Com");

    let definition = ops::get_param_definition::execute(
        &config,
        &json!({"module": "Com", "params": "ComBitPosition"}),
    )
    .expect("get definition");
    assert_eq!(
        definition["definitions"][0]["definition_ref"],
        "/MICROSAR/Com/ComConfig/ComSignal/ComBitPosition"
    );
    assert_eq!(
        definition["definitions"][0]["value_tag"],
        "ECUC-NUMERICAL-PARAM-VALUE"
    );

    let template = ops::find_module_template::execute(
        &config,
        &json!({"module": "Com", "details": true}),
    )
        .expect("find module template");
    let definitions = template["definitions"].as_array().expect("definitions");
    let container = definitions
        .iter()
        .find(|item| item["name"] == "ComSignal")
        .expect("ComSignal definition");
    assert_eq!(container["range"], json!({}));
    assert!(container["ref_target"].is_null());
    let parameter = definitions
        .iter()
        .find(|item| item["name"] == "ComBitPosition")
        .expect("ComBitPosition definition");
    assert_eq!(parameter["range"]["max"], "65535");
    let choice = definitions
        .iter()
        .find(|item| item["name"] == "ComGwDestination")
        .expect("choice container definition");
    assert_eq!(choice["group"], "containers");
    assert_eq!(choice["value_tag"], "ECUC-CONTAINER-VALUE");
    assert_eq!(choice["range"], json!({"lower_multiplicity": "0"}));
    let choice_child = definitions
        .iter()
        .find(|item| item["name"] == "ComGwSignal")
        .expect("choice child definition");
    assert_eq!(
        choice_child["definition_ref"],
        "/MICROSAR/Com/ComConfig/ComGwDestination/ComGwSignal"
    );

    let compact = ops::find_module_template::execute(&config, &json!({"module": "Com"}))
        .expect("compact module template");
    assert!(compact.get("definitions").is_none());
    assert_eq!(compact["containers"][0]["name"], "ComConfig");
    assert_eq!(
        compact["containers"][0]["subcontainers"][0]["name"],
        "ComGwDestination"
    );
    assert_eq!(
        compact["containers"][0]["subcontainers"][1]["name"],
        "ComSignal"
    );
}

#[test]
fn template_cache_invalidates_when_the_definition_file_changes() {
    let (_root, config) = fixture();
    ops::get_param_definition::execute(
        &config,
        &json!({"module": "Com", "params": "ComBitPosition"}),
    )
    .expect("prime template cache");

    let template_path = config.tool_path.join("BSWMD/Com/Com_bswmd.arxml");
    let raw = fs::read_to_string(&template_path).expect("read template");
    let updated = raw.replacen(
        "</PARAMETERS>",
        "<ECUC-INTEGER-PARAM-DEF><SHORT-NAME>ComAddedAfterCache</SHORT-NAME><MAX>9</MAX></ECUC-INTEGER-PARAM-DEF></PARAMETERS>",
        1,
    );
    fs::write(&template_path, updated).expect("update template");

    let definition = ops::get_param_definition::execute(
        &config,
        &json!({"module": "Com", "params": "ComAddedAfterCache"}),
    )
    .expect("reload changed template");
    assert_eq!(definition["definitions"][0]["range"]["max"], "9");
}

#[test]
fn locates_container_by_definition_and_name() {
    let (_root, config) = fixture();
    let result = ops::locate_container::execute(
        &config,
        &json!({
            "module": "Com",
            "definition_ref": "/MICROSAR/Com/ComConfig/ComSignal",
            "short_name_regex": "^SignalA$"
        }),
    )
    .expect("locate");
    assert_eq!(result["count"], 1);
    assert_eq!(result["containers"][0]["short_name"], "SignalA");
    assert!(
        result["containers"][0]["start_line"].as_u64().unwrap()
            < result["containers"][0]["end_line"].as_u64().unwrap()
    );
}

#[test]
fn locates_container_when_xml_tags_are_wrapped_across_lines() {
    let (_root, config) = fixture();
    let file = config.project_path.join("Config/ECUC/Test_Com_ecuc.arxml");
    let raw = fs::read_to_string(&file).expect("read config");
    let wrapped = raw
        .replace(
            "<SHORT-NAME>SignalA</SHORT-NAME>",
            "<SHORT-NAME>\n              SignalA\n            </SHORT-NAME>",
        )
        .replace(
            "<DEFINITION-REF DEST=\"ECUC-PARAM-CONF-CONTAINER-DEF\">/MICROSAR/Com/ComConfig/ComSignal</DEFINITION-REF>",
            "<DEFINITION-REF\n              DEST=\"ECUC-PARAM-CONF-CONTAINER-DEF\">\n              /MICROSAR/Com/ComConfig/ComSignal\n            </DEFINITION-REF>",
        );
    fs::write(&file, wrapped).expect("write wrapped config");

    let result = ops::locate_container::execute(
        &config,
        &json!({
            "module": "Com",
            "definition_ref": "/MICROSAR/Com/ComConfig/ComSignal",
            "short_name_regex": "^SignalA$"
        }),
    )
    .expect("locate wrapped XML");
    assert_eq!(result["count"], 1);
    assert_eq!(result["containers"][0]["short_name"], "SignalA");
}

#[test]
fn inspects_configured_container_values_without_starting_davinci() {
    let (_root, config) = fixture();
    let result = ops::inspect_ecuc_containers::execute(
        &config,
        &json!({
            "module": "Com",
            "container": "ComSignal",
            "short_name_regex": "^SignalA$",
            "params": ["ComBitPosition"]
        }),
    )
    .expect("inspect");

    assert_eq!(result.as_array().expect("array").len(), 1);
    assert_eq!(result[0]["short_name"], "SignalA");
    assert_eq!(result[0]["container_path"], "/ComConfig/SignalA");
    assert_eq!(result[0]["values"]["ComBitPosition"], "8");
}

#[test]
fn inspects_by_full_definition_ref_and_comma_separated_params() {
    let (_root, config) = fixture();
    let result = ops::inspect_ecuc_containers::execute(
        &config,
        &json!({
            "module": "Com",
            "definition_ref": "/MICROSAR/Com/ComConfig/ComSignal",
            "params": "ComBitPosition, MissingParameter"
        }),
    )
    .expect("inspect");

    assert_eq!(
        result[0]["definition_ref"],
        "/MICROSAR/Com/ComConfig/ComSignal"
    );
    assert_eq!(result[0]["values"]["ComBitPosition"], "8");
    assert!(result[0]["values"].get("MissingParameter").is_none());
}

#[test]
fn aggregates_multiple_inspection_requests_into_one_result_array() {
    let (_root, config) = fixture();
    let raw = serde_json::to_string(&json!([
        {
            "func": "inspect_ecuc_containers",
            "module": "Com",
            "container": "ComSignal"
        },
        {
            "func": "inspect_ecuc_containers",
            "module": "Com",
            "definition_ref": "/MICROSAR/Com/ComConfig/ComSignal"
        }
    ]))
    .expect("request JSON");
    let result = CommandDispatcher::new()
        .dispatch_batch(&config, &raw)
        .expect("batch inspect");

    assert_eq!(result.as_array().expect("flat array").len(), 2);
    assert_eq!(result[0]["short_name"], "SignalA");
    assert_eq!(result[1]["short_name"], "SignalA");
}

#[test]
fn doctor_rejects_missing_required_fields_before_starting_davinci() {
    let (_root, config) = fixture();
    let missing_query_module =
        CommandDispatcher::validate_batch(&config, r#"{"func":"find_module"}"#)
            .expect_err("missing query module must fail");
    assert!(missing_query_module
        .to_string()
        .contains("module is required"));

    let implicit_full_generation =
        CommandDispatcher::validate_batch(&config, r#"{"func":"generate_code"}"#)
            .expect("omitted generation module must keep legacy module=all behavior");
    assert_eq!(implicit_full_generation, vec!["generate_code"]);

    let mixed_inspection = CommandDispatcher::validate_batch(
        &config,
        r#"[{"func":"inspect_ecuc_containers","module":"Com"},{"func":"find_module","module":"Com"}]"#,
    )
    .expect_err("doctor must reject the same mixed batch as execution");
    assert!(mixed_inspection.to_string().contains("cannot be mixed"));
}

#[test]
fn batch_validates_every_item_before_applying_an_edit() {
    let (_root, config) = fixture();
    let file = config.project_path.join("batch-edit.arxml");
    fs::write(&file, "one\ntwo\nthree\n").expect("write batch edit file");
    let raw = json!([
        {
            "func": "edit_file",
            "path": file,
            "expected": {"2": "two"},
            "edits": {"2": "TWO"}
        },
        {"func": "find_module"}
    ])
    .to_string();

    let error = CommandDispatcher::new()
        .dispatch_batch(&config, &raw)
        .expect_err("a mutating item must reject a multi-item batch before editing");
    assert!(error.to_string().contains("must be standalone requests"));
    assert_eq!(
        fs::read_to_string(config.project_path.join("batch-edit.arxml"))
            .expect("read unmodified batch file"),
        "one\ntwo\nthree\n"
    );
}

#[test]
fn doctor_rejects_generation_inside_a_multi_item_batch() {
    let (_root, config) = fixture();
    let error = CommandDispatcher::validate_batch(
        &config,
        r#"[{"func":"find_module","module":"Com"},{"func":"generate_code","module":"Com"}]"#,
    )
    .expect_err("generation in a multi-item batch must be rejected before execution");

    assert!(error.to_string().contains("must be standalone requests"));
}

#[test]
fn edits_only_requested_line_and_preserves_crlf() {
    let (_root, config) = fixture();
    let file = config.project_path.join("edit.arxml");
    fs::write(&file, b"one\r\ntwo\r\nthree\r\n").expect("write edit file");
    let result = ops::edit_file::execute(
        &config,
        &json!({
            "path": file,
            "expected": {"2": "two"},
            "edits": {"2": "TWO"}
        }),
    )
    .expect("edit");
    assert_eq!(result["applied_edits"], 1);
    assert_eq!(
        fs::read_to_string(config.project_path.join("edit.arxml")).expect("read edited"),
        "one\r\nTWO\r\nthree\r\n"
    );
}

#[test]
fn refuses_edit_when_inspected_text_is_stale() {
    let (_root, config) = fixture();
    let file = config.project_path.join("stale.arxml");
    fs::write(&file, "one\nchanged elsewhere\nthree\n").expect("write stale file");
    let error = ops::edit_file::execute(
        &config,
        &json!({
            "path": file,
            "expected": {"2": "two"},
            "edits": {"2": "TWO"}
        }),
    )
    .expect_err("stale edit must fail");
    assert!(error.to_string().contains("file changed"));
    assert_eq!(
        fs::read_to_string(config.project_path.join("stale.arxml")).expect("read unchanged"),
        "one\nchanged elsewhere\nthree\n"
    );
}

#[test]
fn refuses_edit_outside_project() {
    let (root, config) = fixture();
    let outside = root.path().join("outside.arxml");
    fs::write(&outside, "one\n").expect("outside");
    let error = ops::edit_file::execute(
        &config,
        &json!({"path": outside, "edits": {"1": "changed"}}),
    )
    .expect_err("outside edit must fail");
    assert!(error.to_string().contains("outside project_path"));
}

#[cfg(windows)]
#[test]
fn session_paths_are_accepted_by_legacy_java_tools() {
    let (root, _config) = fixture();
    let project = root.path().join("Cfg");
    let tool = root.path().join("SIP");
    fs::write(
        project.join("lgk-vector.json"),
        format!(
            "{{\"project_path\":{},\"tool_path\":{}}}",
            serde_json::to_string(&project).expect("project JSON"),
            serde_json::to_string(&tool).expect("tool JSON")
        ),
    )
    .expect("config file");

    let config = SessionConfig::load(&project).expect("load session");
    assert!(!config.project_path.to_string_lossy().starts_with(r"\\?\"));
    assert!(!config
        .dpa_file()
        .expect("dpa")
        .to_string_lossy()
        .starts_with(r"\\?\"));
}

#[test]
fn accepts_legacy_lgk_config_field_names() {
    let (root, expected) = fixture();
    let project = root.path().join("Cfg");
    let tool = root.path().join("SIP");
    fs::write(
        project.join("lgk-vector.json"),
        format!(
            "{{\"LGK_project_path\":{},\"LGK_tool_path\":{}}}",
            serde_json::to_string(&project).expect("project JSON"),
            serde_json::to_string(&tool).expect("tool JSON")
        ),
    )
    .expect("legacy config file");

    let config = SessionConfig::load(&project).expect("load legacy session");
    assert_eq!(config.project_path, expected.project_path);
    assert_eq!(config.tool_path, expected.tool_path);
}

#[test]
fn supports_non_microsar_definitions_and_explicit_tool_selection() {
    let root = tempdir().expect("tempdir");
    let project = root.path().join("GenericCfg");
    let tool = root.path().join("VendorSip");
    let config_file = project.join("Config/ECUC/Vendor_Nm_ecuc.arxml");
    let project_file = project.join("GenericPlatform.dpa");
    let command = tool.join("DaVinci/Exec/DVCfgCmd.exe");
    fs::create_dir_all(config_file.parent().expect("config parent")).expect("project dirs");
    fs::create_dir_all(command.parent().expect("command parent")).expect("command dirs");
    fs::create_dir_all(tool.join("Definitions/Networking")).expect("definition dirs");
    fs::write(&command, b"").expect("command placeholder");
    fs::write(project.join("ArchivedProject.dpa"), "<ProjectAssistant/>").expect("other dpa");
    fs::write(
        &project_file,
        r#"<?xml version="1.0"?>
<ProjectAssistant>
  <EcucSplitter>
    <Splitter File=".\Config\ECUC\Vendor_Nm_ecuc.arxml">
      <Module Name="Nm"/>
    </Splitter>
  </EcucSplitter>
</ProjectAssistant>"#,
    )
    .expect("selected dpa");
    fs::write(
        &config_file,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<AUTOSAR>
  <ECUC-MODULE-CONFIGURATION-VALUES>
    <SHORT-NAME>NetworkManagerInstance</SHORT-NAME>
    <DEFINITION-REF DEST="ECUC-MODULE-DEF">/AcmeAutosar/Nm</DEFINITION-REF>
  </ECUC-MODULE-CONFIGURATION-VALUES>
</AUTOSAR>"#,
    )
    .expect("module config");
    fs::write(
        tool.join("Definitions/Networking/Nm_definition.arxml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<AUTOSAR>
  <AR-PACKAGES>
    <AR-PACKAGE>
      <SHORT-NAME>AcmeAutosar</SHORT-NAME>
      <ELEMENTS>
        <ECUC-MODULE-DEF>
          <SHORT-NAME>Nm</SHORT-NAME>
          <PARAMETERS>
            <ECUC-FLOAT-PARAM-DEF>
              <SHORT-NAME>NmMainFunctionPeriod</SHORT-NAME>
              <DEFAULT-VALUE>0.01</DEFAULT-VALUE>
            </ECUC-FLOAT-PARAM-DEF>
          </PARAMETERS>
        </ECUC-MODULE-DEF>
      </ELEMENTS>
    </AR-PACKAGE>
  </AR-PACKAGES>
</AUTOSAR>"#,
    )
    .expect("vendor definition");
    fs::write(
        project.join("lgk-vector.json"),
        format!(
            "{{\"tool_path\":{},\"project_file\":\"GenericPlatform.dpa\",\"davinci_command_path\":\"DaVinci/Exec/DVCfgCmd.exe\"}}",
            serde_json::to_string(&tool).expect("tool JSON"),
        ),
    )
    .expect("bridge config");

    let config = SessionConfig::load(&project).expect("load generic session");
    assert_eq!(config.dpa_file().expect("selected dpa"), project_file);
    assert_eq!(
        config.davinci_command_path.as_deref(),
        Some(command.as_path())
    );

    let module = ops::find_module::execute(&config, &json!({"module": "Nm"})).expect("find module");
    assert_eq!(module["definition_ref"], "/AcmeAutosar/Nm");
    let definition = ops::get_param_definition::execute(
        &config,
        &json!({"module": "Nm", "params": "NmMainFunctionPeriod"}),
    )
    .expect("vendor parameter definition");
    assert_eq!(
        definition["definitions"][0]["definition_ref"],
        "/AcmeAutosar/Nm/NmMainFunctionPeriod"
    );
}
