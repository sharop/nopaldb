// Code generated by protoc-gen-go. DO NOT EDIT.
// versions:
// 	protoc-gen-go v1.27.1
// 	protoc        v3.19.4
// source: pb/v1/log/log.proto

package log_v1

import (
	protoreflect "google.golang.org/protobuf/reflect/protoreflect"
	protoimpl "google.golang.org/protobuf/runtime/protoimpl"
	reflect "reflect"
	sync "sync"
)

const (
	// Verify that this generated code is sufficiently up-to-date.
	_ = protoimpl.EnforceVersion(20 - protoimpl.MinVersion)
	// Verify that runtime/protoimpl is sufficiently up-to-date.
	_ = protoimpl.EnforceVersion(protoimpl.MaxVersion - 20)
)

type Item_Level int32

const (
	Item_CORE      Item_Level = 0
	Item_ENGINE    Item_Level = 1
	Item_SERVICES  Item_Level = 2
	Item_RESOURCES Item_Level = 3
)

// Enum value maps for Item_Level.
var (
	Item_Level_name = map[int32]string{
		0: "CORE",
		1: "ENGINE",
		2: "SERVICES",
		3: "RESOURCES",
	}
	Item_Level_value = map[string]int32{
		"CORE":      0,
		"ENGINE":    1,
		"SERVICES":  2,
		"RESOURCES": 3,
	}
)

func (x Item_Level) Enum() *Item_Level {
	p := new(Item_Level)
	*p = x
	return p
}

func (x Item_Level) String() string {
	return protoimpl.X.EnumStringOf(x.Descriptor(), protoreflect.EnumNumber(x))
}

func (Item_Level) Descriptor() protoreflect.EnumDescriptor {
	return file_pb_v1_log_log_proto_enumTypes[0].Descriptor()
}

func (Item_Level) Type() protoreflect.EnumType {
	return &file_pb_v1_log_log_proto_enumTypes[0]
}

func (x Item_Level) Number() protoreflect.EnumNumber {
	return protoreflect.EnumNumber(x)
}

// Deprecated: Use Item_Level.Descriptor instead.
func (Item_Level) EnumDescriptor() ([]byte, []int) {
	return file_pb_v1_log_log_proto_rawDescGZIP(), []int{10, 0}
}

type Item_Resource int32

const (
	Item_SOURCE Item_Resource = 0
	Item_SPACE  Item_Resource = 1
	Item_ASSET  Item_Resource = 2
)

// Enum value maps for Item_Resource.
var (
	Item_Resource_name = map[int32]string{
		0: "SOURCE",
		1: "SPACE",
		2: "ASSET",
	}
	Item_Resource_value = map[string]int32{
		"SOURCE": 0,
		"SPACE":  1,
		"ASSET":  2,
	}
)

func (x Item_Resource) Enum() *Item_Resource {
	p := new(Item_Resource)
	*p = x
	return p
}

func (x Item_Resource) String() string {
	return protoimpl.X.EnumStringOf(x.Descriptor(), protoreflect.EnumNumber(x))
}

func (Item_Resource) Descriptor() protoreflect.EnumDescriptor {
	return file_pb_v1_log_log_proto_enumTypes[1].Descriptor()
}

func (Item_Resource) Type() protoreflect.EnumType {
	return &file_pb_v1_log_log_proto_enumTypes[1]
}

func (x Item_Resource) Number() protoreflect.EnumNumber {
	return protoreflect.EnumNumber(x)
}

// Deprecated: Use Item_Resource.Descriptor instead.
func (Item_Resource) EnumDescriptor() ([]byte, []int) {
	return file_pb_v1_log_log_proto_rawDescGZIP(), []int{10, 1}
}

type GetServersRequest struct {
	state         protoimpl.MessageState
	sizeCache     protoimpl.SizeCache
	unknownFields protoimpl.UnknownFields
}

func (x *GetServersRequest) Reset() {
	*x = GetServersRequest{}
	if protoimpl.UnsafeEnabled {
		mi := &file_pb_v1_log_log_proto_msgTypes[0]
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		ms.StoreMessageInfo(mi)
	}
}

func (x *GetServersRequest) String() string {
	return protoimpl.X.MessageStringOf(x)
}

func (*GetServersRequest) ProtoMessage() {}

func (x *GetServersRequest) ProtoReflect() protoreflect.Message {
	mi := &file_pb_v1_log_log_proto_msgTypes[0]
	if protoimpl.UnsafeEnabled && x != nil {
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		if ms.LoadMessageInfo() == nil {
			ms.StoreMessageInfo(mi)
		}
		return ms
	}
	return mi.MessageOf(x)
}

// Deprecated: Use GetServersRequest.ProtoReflect.Descriptor instead.
func (*GetServersRequest) Descriptor() ([]byte, []int) {
	return file_pb_v1_log_log_proto_rawDescGZIP(), []int{0}
}

type GetServersResponse struct {
	state         protoimpl.MessageState
	sizeCache     protoimpl.SizeCache
	unknownFields protoimpl.UnknownFields

	Servers []*Server `protobuf:"bytes,1,rep,name=servers,proto3" json:"servers,omitempty"`
}

func (x *GetServersResponse) Reset() {
	*x = GetServersResponse{}
	if protoimpl.UnsafeEnabled {
		mi := &file_pb_v1_log_log_proto_msgTypes[1]
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		ms.StoreMessageInfo(mi)
	}
}

func (x *GetServersResponse) String() string {
	return protoimpl.X.MessageStringOf(x)
}

func (*GetServersResponse) ProtoMessage() {}

func (x *GetServersResponse) ProtoReflect() protoreflect.Message {
	mi := &file_pb_v1_log_log_proto_msgTypes[1]
	if protoimpl.UnsafeEnabled && x != nil {
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		if ms.LoadMessageInfo() == nil {
			ms.StoreMessageInfo(mi)
		}
		return ms
	}
	return mi.MessageOf(x)
}

// Deprecated: Use GetServersResponse.ProtoReflect.Descriptor instead.
func (*GetServersResponse) Descriptor() ([]byte, []int) {
	return file_pb_v1_log_log_proto_rawDescGZIP(), []int{1}
}

func (x *GetServersResponse) GetServers() []*Server {
	if x != nil {
		return x.Servers
	}
	return nil
}

type Server struct {
	state         protoimpl.MessageState
	sizeCache     protoimpl.SizeCache
	unknownFields protoimpl.UnknownFields

	Id       string `protobuf:"bytes,1,opt,name=id,proto3" json:"id,omitempty"`
	RpcAddr  string `protobuf:"bytes,2,opt,name=rpc_addr,json=rpcAddr,proto3" json:"rpc_addr,omitempty"`
	IsLeader bool   `protobuf:"varint,3,opt,name=is_leader,json=isLeader,proto3" json:"is_leader,omitempty"`
}

func (x *Server) Reset() {
	*x = Server{}
	if protoimpl.UnsafeEnabled {
		mi := &file_pb_v1_log_log_proto_msgTypes[2]
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		ms.StoreMessageInfo(mi)
	}
}

func (x *Server) String() string {
	return protoimpl.X.MessageStringOf(x)
}

func (*Server) ProtoMessage() {}

func (x *Server) ProtoReflect() protoreflect.Message {
	mi := &file_pb_v1_log_log_proto_msgTypes[2]
	if protoimpl.UnsafeEnabled && x != nil {
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		if ms.LoadMessageInfo() == nil {
			ms.StoreMessageInfo(mi)
		}
		return ms
	}
	return mi.MessageOf(x)
}

// Deprecated: Use Server.ProtoReflect.Descriptor instead.
func (*Server) Descriptor() ([]byte, []int) {
	return file_pb_v1_log_log_proto_rawDescGZIP(), []int{2}
}

func (x *Server) GetId() string {
	if x != nil {
		return x.Id
	}
	return ""
}

func (x *Server) GetRpcAddr() string {
	if x != nil {
		return x.RpcAddr
	}
	return ""
}

func (x *Server) GetIsLeader() bool {
	if x != nil {
		return x.IsLeader
	}
	return false
}

type ProduceRequest struct {
	state         protoimpl.MessageState
	sizeCache     protoimpl.SizeCache
	unknownFields protoimpl.UnknownFields

	Record *Record `protobuf:"bytes,1,opt,name=record,proto3" json:"record,omitempty"`
}

func (x *ProduceRequest) Reset() {
	*x = ProduceRequest{}
	if protoimpl.UnsafeEnabled {
		mi := &file_pb_v1_log_log_proto_msgTypes[3]
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		ms.StoreMessageInfo(mi)
	}
}

func (x *ProduceRequest) String() string {
	return protoimpl.X.MessageStringOf(x)
}

func (*ProduceRequest) ProtoMessage() {}

func (x *ProduceRequest) ProtoReflect() protoreflect.Message {
	mi := &file_pb_v1_log_log_proto_msgTypes[3]
	if protoimpl.UnsafeEnabled && x != nil {
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		if ms.LoadMessageInfo() == nil {
			ms.StoreMessageInfo(mi)
		}
		return ms
	}
	return mi.MessageOf(x)
}

// Deprecated: Use ProduceRequest.ProtoReflect.Descriptor instead.
func (*ProduceRequest) Descriptor() ([]byte, []int) {
	return file_pb_v1_log_log_proto_rawDescGZIP(), []int{3}
}

func (x *ProduceRequest) GetRecord() *Record {
	if x != nil {
		return x.Record
	}
	return nil
}

type ProduceResponse struct {
	state         protoimpl.MessageState
	sizeCache     protoimpl.SizeCache
	unknownFields protoimpl.UnknownFields

	Key string `protobuf:"bytes,1,opt,name=key,proto3" json:"key,omitempty"`
}

func (x *ProduceResponse) Reset() {
	*x = ProduceResponse{}
	if protoimpl.UnsafeEnabled {
		mi := &file_pb_v1_log_log_proto_msgTypes[4]
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		ms.StoreMessageInfo(mi)
	}
}

func (x *ProduceResponse) String() string {
	return protoimpl.X.MessageStringOf(x)
}

func (*ProduceResponse) ProtoMessage() {}

func (x *ProduceResponse) ProtoReflect() protoreflect.Message {
	mi := &file_pb_v1_log_log_proto_msgTypes[4]
	if protoimpl.UnsafeEnabled && x != nil {
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		if ms.LoadMessageInfo() == nil {
			ms.StoreMessageInfo(mi)
		}
		return ms
	}
	return mi.MessageOf(x)
}

// Deprecated: Use ProduceResponse.ProtoReflect.Descriptor instead.
func (*ProduceResponse) Descriptor() ([]byte, []int) {
	return file_pb_v1_log_log_proto_rawDescGZIP(), []int{4}
}

func (x *ProduceResponse) GetKey() string {
	if x != nil {
		return x.Key
	}
	return ""
}

type ConsumeRequest struct {
	state         protoimpl.MessageState
	sizeCache     protoimpl.SizeCache
	unknownFields protoimpl.UnknownFields

	Key string `protobuf:"bytes,1,opt,name=key,proto3" json:"key,omitempty"`
}

func (x *ConsumeRequest) Reset() {
	*x = ConsumeRequest{}
	if protoimpl.UnsafeEnabled {
		mi := &file_pb_v1_log_log_proto_msgTypes[5]
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		ms.StoreMessageInfo(mi)
	}
}

func (x *ConsumeRequest) String() string {
	return protoimpl.X.MessageStringOf(x)
}

func (*ConsumeRequest) ProtoMessage() {}

func (x *ConsumeRequest) ProtoReflect() protoreflect.Message {
	mi := &file_pb_v1_log_log_proto_msgTypes[5]
	if protoimpl.UnsafeEnabled && x != nil {
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		if ms.LoadMessageInfo() == nil {
			ms.StoreMessageInfo(mi)
		}
		return ms
	}
	return mi.MessageOf(x)
}

// Deprecated: Use ConsumeRequest.ProtoReflect.Descriptor instead.
func (*ConsumeRequest) Descriptor() ([]byte, []int) {
	return file_pb_v1_log_log_proto_rawDescGZIP(), []int{5}
}

func (x *ConsumeRequest) GetKey() string {
	if x != nil {
		return x.Key
	}
	return ""
}

type ConsumeResponse struct {
	state         protoimpl.MessageState
	sizeCache     protoimpl.SizeCache
	unknownFields protoimpl.UnknownFields

	Record *Record `protobuf:"bytes,2,opt,name=record,proto3" json:"record,omitempty"`
}

func (x *ConsumeResponse) Reset() {
	*x = ConsumeResponse{}
	if protoimpl.UnsafeEnabled {
		mi := &file_pb_v1_log_log_proto_msgTypes[6]
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		ms.StoreMessageInfo(mi)
	}
}

func (x *ConsumeResponse) String() string {
	return protoimpl.X.MessageStringOf(x)
}

func (*ConsumeResponse) ProtoMessage() {}

func (x *ConsumeResponse) ProtoReflect() protoreflect.Message {
	mi := &file_pb_v1_log_log_proto_msgTypes[6]
	if protoimpl.UnsafeEnabled && x != nil {
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		if ms.LoadMessageInfo() == nil {
			ms.StoreMessageInfo(mi)
		}
		return ms
	}
	return mi.MessageOf(x)
}

// Deprecated: Use ConsumeResponse.ProtoReflect.Descriptor instead.
func (*ConsumeResponse) Descriptor() ([]byte, []int) {
	return file_pb_v1_log_log_proto_rawDescGZIP(), []int{6}
}

func (x *ConsumeResponse) GetRecord() *Record {
	if x != nil {
		return x.Record
	}
	return nil
}

type Record struct {
	state         protoimpl.MessageState
	sizeCache     protoimpl.SizeCache
	unknownFields protoimpl.UnknownFields

	Key   string `protobuf:"bytes,1,opt,name=key,proto3" json:"key,omitempty"`
	Value string `protobuf:"bytes,2,opt,name=value,proto3" json:"value,omitempty"`
}

func (x *Record) Reset() {
	*x = Record{}
	if protoimpl.UnsafeEnabled {
		mi := &file_pb_v1_log_log_proto_msgTypes[7]
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		ms.StoreMessageInfo(mi)
	}
}

func (x *Record) String() string {
	return protoimpl.X.MessageStringOf(x)
}

func (*Record) ProtoMessage() {}

func (x *Record) ProtoReflect() protoreflect.Message {
	mi := &file_pb_v1_log_log_proto_msgTypes[7]
	if protoimpl.UnsafeEnabled && x != nil {
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		if ms.LoadMessageInfo() == nil {
			ms.StoreMessageInfo(mi)
		}
		return ms
	}
	return mi.MessageOf(x)
}

// Deprecated: Use Record.ProtoReflect.Descriptor instead.
func (*Record) Descriptor() ([]byte, []int) {
	return file_pb_v1_log_log_proto_rawDescGZIP(), []int{7}
}

func (x *Record) GetKey() string {
	if x != nil {
		return x.Key
	}
	return ""
}

func (x *Record) GetValue() string {
	if x != nil {
		return x.Value
	}
	return ""
}

type ItemList struct {
	state         protoimpl.MessageState
	sizeCache     protoimpl.SizeCache
	unknownFields protoimpl.UnknownFields

	Items []*Item `protobuf:"bytes,1,rep,name=items,proto3" json:"items,omitempty"`
}

func (x *ItemList) Reset() {
	*x = ItemList{}
	if protoimpl.UnsafeEnabled {
		mi := &file_pb_v1_log_log_proto_msgTypes[8]
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		ms.StoreMessageInfo(mi)
	}
}

func (x *ItemList) String() string {
	return protoimpl.X.MessageStringOf(x)
}

func (*ItemList) ProtoMessage() {}

func (x *ItemList) ProtoReflect() protoreflect.Message {
	mi := &file_pb_v1_log_log_proto_msgTypes[8]
	if protoimpl.UnsafeEnabled && x != nil {
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		if ms.LoadMessageInfo() == nil {
			ms.StoreMessageInfo(mi)
		}
		return ms
	}
	return mi.MessageOf(x)
}

// Deprecated: Use ItemList.ProtoReflect.Descriptor instead.
func (*ItemList) Descriptor() ([]byte, []int) {
	return file_pb_v1_log_log_proto_rawDescGZIP(), []int{8}
}

func (x *ItemList) GetItems() []*Item {
	if x != nil {
		return x.Items
	}
	return nil
}

type Connections struct {
	state         protoimpl.MessageState
	sizeCache     protoimpl.SizeCache
	unknownFields protoimpl.UnknownFields

	ID   string `protobuf:"bytes,1,opt,name=ID,proto3" json:"ID,omitempty"`
	Path string `protobuf:"bytes,2,opt,name=Path,proto3" json:"Path,omitempty"`
}

func (x *Connections) Reset() {
	*x = Connections{}
	if protoimpl.UnsafeEnabled {
		mi := &file_pb_v1_log_log_proto_msgTypes[9]
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		ms.StoreMessageInfo(mi)
	}
}

func (x *Connections) String() string {
	return protoimpl.X.MessageStringOf(x)
}

func (*Connections) ProtoMessage() {}

func (x *Connections) ProtoReflect() protoreflect.Message {
	mi := &file_pb_v1_log_log_proto_msgTypes[9]
	if protoimpl.UnsafeEnabled && x != nil {
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		if ms.LoadMessageInfo() == nil {
			ms.StoreMessageInfo(mi)
		}
		return ms
	}
	return mi.MessageOf(x)
}

// Deprecated: Use Connections.ProtoReflect.Descriptor instead.
func (*Connections) Descriptor() ([]byte, []int) {
	return file_pb_v1_log_log_proto_rawDescGZIP(), []int{9}
}

func (x *Connections) GetID() string {
	if x != nil {
		return x.ID
	}
	return ""
}

func (x *Connections) GetPath() string {
	if x != nil {
		return x.Path
	}
	return ""
}

type Item struct {
	state         protoimpl.MessageState
	sizeCache     protoimpl.SizeCache
	unknownFields protoimpl.UnknownFields

	ID       string        `protobuf:"bytes,1,opt,name=ID,proto3" json:"ID,omitempty"`
	Path     string        `protobuf:"bytes,2,opt,name=path,proto3" json:"path,omitempty"`
	Level    Item_Level    `protobuf:"varint,3,opt,name=level,proto3,enum=log.v1.Item_Level" json:"level,omitempty"`
	Type     Item_Resource `protobuf:"varint,4,opt,name=type,proto3,enum=log.v1.Item_Resource" json:"type,omitempty"`
	Key      string        `protobuf:"bytes,5,opt,name=key,proto3" json:"key,omitempty"`
	Value    string        `protobuf:"bytes,6,opt,name=value,proto3" json:"value,omitempty"`
	Created  int64         `protobuf:"varint,7,opt,name=created,proto3" json:"created,omitempty"`
	Modified int64         `protobuf:"varint,8,opt,name=modified,proto3" json:"modified,omitempty"`
	Version  uint64        `protobuf:"varint,9,opt,name=version,proto3" json:"version,omitempty"`
	Meta     []byte        `protobuf:"bytes,10,opt,name=meta,proto3" json:"meta,omitempty"`
	UserMeta []byte        `protobuf:"bytes,11,opt,name=user_meta,json=userMeta,proto3" json:"user_meta,omitempty"`
}

func (x *Item) Reset() {
	*x = Item{}
	if protoimpl.UnsafeEnabled {
		mi := &file_pb_v1_log_log_proto_msgTypes[10]
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		ms.StoreMessageInfo(mi)
	}
}

func (x *Item) String() string {
	return protoimpl.X.MessageStringOf(x)
}

func (*Item) ProtoMessage() {}

func (x *Item) ProtoReflect() protoreflect.Message {
	mi := &file_pb_v1_log_log_proto_msgTypes[10]
	if protoimpl.UnsafeEnabled && x != nil {
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		if ms.LoadMessageInfo() == nil {
			ms.StoreMessageInfo(mi)
		}
		return ms
	}
	return mi.MessageOf(x)
}

// Deprecated: Use Item.ProtoReflect.Descriptor instead.
func (*Item) Descriptor() ([]byte, []int) {
	return file_pb_v1_log_log_proto_rawDescGZIP(), []int{10}
}

func (x *Item) GetID() string {
	if x != nil {
		return x.ID
	}
	return ""
}

func (x *Item) GetPath() string {
	if x != nil {
		return x.Path
	}
	return ""
}

func (x *Item) GetLevel() Item_Level {
	if x != nil {
		return x.Level
	}
	return Item_CORE
}

func (x *Item) GetType() Item_Resource {
	if x != nil {
		return x.Type
	}
	return Item_SOURCE
}

func (x *Item) GetKey() string {
	if x != nil {
		return x.Key
	}
	return ""
}

func (x *Item) GetValue() string {
	if x != nil {
		return x.Value
	}
	return ""
}

func (x *Item) GetCreated() int64 {
	if x != nil {
		return x.Created
	}
	return 0
}

func (x *Item) GetModified() int64 {
	if x != nil {
		return x.Modified
	}
	return 0
}

func (x *Item) GetVersion() uint64 {
	if x != nil {
		return x.Version
	}
	return 0
}

func (x *Item) GetMeta() []byte {
	if x != nil {
		return x.Meta
	}
	return nil
}

func (x *Item) GetUserMeta() []byte {
	if x != nil {
		return x.UserMeta
	}
	return nil
}

var File_pb_v1_log_log_proto protoreflect.FileDescriptor

var file_pb_v1_log_log_proto_rawDesc = []byte{
	0x0a, 0x13, 0x70, 0x62, 0x2f, 0x76, 0x31, 0x2f, 0x6c, 0x6f, 0x67, 0x2f, 0x6c, 0x6f, 0x67, 0x2e,
	0x70, 0x72, 0x6f, 0x74, 0x6f, 0x12, 0x06, 0x6c, 0x6f, 0x67, 0x2e, 0x76, 0x31, 0x22, 0x13, 0x0a,
	0x11, 0x47, 0x65, 0x74, 0x53, 0x65, 0x72, 0x76, 0x65, 0x72, 0x73, 0x52, 0x65, 0x71, 0x75, 0x65,
	0x73, 0x74, 0x22, 0x3e, 0x0a, 0x12, 0x47, 0x65, 0x74, 0x53, 0x65, 0x72, 0x76, 0x65, 0x72, 0x73,
	0x52, 0x65, 0x73, 0x70, 0x6f, 0x6e, 0x73, 0x65, 0x12, 0x28, 0x0a, 0x07, 0x73, 0x65, 0x72, 0x76,
	0x65, 0x72, 0x73, 0x18, 0x01, 0x20, 0x03, 0x28, 0x0b, 0x32, 0x0e, 0x2e, 0x6c, 0x6f, 0x67, 0x2e,
	0x76, 0x31, 0x2e, 0x53, 0x65, 0x72, 0x76, 0x65, 0x72, 0x52, 0x07, 0x73, 0x65, 0x72, 0x76, 0x65,
	0x72, 0x73, 0x22, 0x50, 0x0a, 0x06, 0x53, 0x65, 0x72, 0x76, 0x65, 0x72, 0x12, 0x0e, 0x0a, 0x02,
	0x69, 0x64, 0x18, 0x01, 0x20, 0x01, 0x28, 0x09, 0x52, 0x02, 0x69, 0x64, 0x12, 0x19, 0x0a, 0x08,
	0x72, 0x70, 0x63, 0x5f, 0x61, 0x64, 0x64, 0x72, 0x18, 0x02, 0x20, 0x01, 0x28, 0x09, 0x52, 0x07,
	0x72, 0x70, 0x63, 0x41, 0x64, 0x64, 0x72, 0x12, 0x1b, 0x0a, 0x09, 0x69, 0x73, 0x5f, 0x6c, 0x65,
	0x61, 0x64, 0x65, 0x72, 0x18, 0x03, 0x20, 0x01, 0x28, 0x08, 0x52, 0x08, 0x69, 0x73, 0x4c, 0x65,
	0x61, 0x64, 0x65, 0x72, 0x22, 0x38, 0x0a, 0x0e, 0x50, 0x72, 0x6f, 0x64, 0x75, 0x63, 0x65, 0x52,
	0x65, 0x71, 0x75, 0x65, 0x73, 0x74, 0x12, 0x26, 0x0a, 0x06, 0x72, 0x65, 0x63, 0x6f, 0x72, 0x64,
	0x18, 0x01, 0x20, 0x01, 0x28, 0x0b, 0x32, 0x0e, 0x2e, 0x6c, 0x6f, 0x67, 0x2e, 0x76, 0x31, 0x2e,
	0x52, 0x65, 0x63, 0x6f, 0x72, 0x64, 0x52, 0x06, 0x72, 0x65, 0x63, 0x6f, 0x72, 0x64, 0x22, 0x23,
	0x0a, 0x0f, 0x50, 0x72, 0x6f, 0x64, 0x75, 0x63, 0x65, 0x52, 0x65, 0x73, 0x70, 0x6f, 0x6e, 0x73,
	0x65, 0x12, 0x10, 0x0a, 0x03, 0x6b, 0x65, 0x79, 0x18, 0x01, 0x20, 0x01, 0x28, 0x09, 0x52, 0x03,
	0x6b, 0x65, 0x79, 0x22, 0x22, 0x0a, 0x0e, 0x43, 0x6f, 0x6e, 0x73, 0x75, 0x6d, 0x65, 0x52, 0x65,
	0x71, 0x75, 0x65, 0x73, 0x74, 0x12, 0x10, 0x0a, 0x03, 0x6b, 0x65, 0x79, 0x18, 0x01, 0x20, 0x01,
	0x28, 0x09, 0x52, 0x03, 0x6b, 0x65, 0x79, 0x22, 0x39, 0x0a, 0x0f, 0x43, 0x6f, 0x6e, 0x73, 0x75,
	0x6d, 0x65, 0x52, 0x65, 0x73, 0x70, 0x6f, 0x6e, 0x73, 0x65, 0x12, 0x26, 0x0a, 0x06, 0x72, 0x65,
	0x63, 0x6f, 0x72, 0x64, 0x18, 0x02, 0x20, 0x01, 0x28, 0x0b, 0x32, 0x0e, 0x2e, 0x6c, 0x6f, 0x67,
	0x2e, 0x76, 0x31, 0x2e, 0x52, 0x65, 0x63, 0x6f, 0x72, 0x64, 0x52, 0x06, 0x72, 0x65, 0x63, 0x6f,
	0x72, 0x64, 0x22, 0x30, 0x0a, 0x06, 0x52, 0x65, 0x63, 0x6f, 0x72, 0x64, 0x12, 0x10, 0x0a, 0x03,
	0x6b, 0x65, 0x79, 0x18, 0x01, 0x20, 0x01, 0x28, 0x09, 0x52, 0x03, 0x6b, 0x65, 0x79, 0x12, 0x14,
	0x0a, 0x05, 0x76, 0x61, 0x6c, 0x75, 0x65, 0x18, 0x02, 0x20, 0x01, 0x28, 0x09, 0x52, 0x05, 0x76,
	0x61, 0x6c, 0x75, 0x65, 0x22, 0x2e, 0x0a, 0x08, 0x49, 0x74, 0x65, 0x6d, 0x4c, 0x69, 0x73, 0x74,
	0x12, 0x22, 0x0a, 0x05, 0x69, 0x74, 0x65, 0x6d, 0x73, 0x18, 0x01, 0x20, 0x03, 0x28, 0x0b, 0x32,
	0x0c, 0x2e, 0x6c, 0x6f, 0x67, 0x2e, 0x76, 0x31, 0x2e, 0x49, 0x74, 0x65, 0x6d, 0x52, 0x05, 0x69,
	0x74, 0x65, 0x6d, 0x73, 0x22, 0x31, 0x0a, 0x0b, 0x43, 0x6f, 0x6e, 0x6e, 0x65, 0x63, 0x74, 0x69,
	0x6f, 0x6e, 0x73, 0x12, 0x0e, 0x0a, 0x02, 0x49, 0x44, 0x18, 0x01, 0x20, 0x01, 0x28, 0x09, 0x52,
	0x02, 0x49, 0x44, 0x12, 0x12, 0x0a, 0x04, 0x50, 0x61, 0x74, 0x68, 0x18, 0x02, 0x20, 0x01, 0x28,
	0x09, 0x52, 0x04, 0x50, 0x61, 0x74, 0x68, 0x22, 0x92, 0x03, 0x0a, 0x04, 0x49, 0x74, 0x65, 0x6d,
	0x12, 0x0e, 0x0a, 0x02, 0x49, 0x44, 0x18, 0x01, 0x20, 0x01, 0x28, 0x09, 0x52, 0x02, 0x49, 0x44,
	0x12, 0x12, 0x0a, 0x04, 0x70, 0x61, 0x74, 0x68, 0x18, 0x02, 0x20, 0x01, 0x28, 0x09, 0x52, 0x04,
	0x70, 0x61, 0x74, 0x68, 0x12, 0x28, 0x0a, 0x05, 0x6c, 0x65, 0x76, 0x65, 0x6c, 0x18, 0x03, 0x20,
	0x01, 0x28, 0x0e, 0x32, 0x12, 0x2e, 0x6c, 0x6f, 0x67, 0x2e, 0x76, 0x31, 0x2e, 0x49, 0x74, 0x65,
	0x6d, 0x2e, 0x4c, 0x65, 0x76, 0x65, 0x6c, 0x52, 0x05, 0x6c, 0x65, 0x76, 0x65, 0x6c, 0x12, 0x29,
	0x0a, 0x04, 0x74, 0x79, 0x70, 0x65, 0x18, 0x04, 0x20, 0x01, 0x28, 0x0e, 0x32, 0x15, 0x2e, 0x6c,
	0x6f, 0x67, 0x2e, 0x76, 0x31, 0x2e, 0x49, 0x74, 0x65, 0x6d, 0x2e, 0x52, 0x65, 0x73, 0x6f, 0x75,
	0x72, 0x63, 0x65, 0x52, 0x04, 0x74, 0x79, 0x70, 0x65, 0x12, 0x10, 0x0a, 0x03, 0x6b, 0x65, 0x79,
	0x18, 0x05, 0x20, 0x01, 0x28, 0x09, 0x52, 0x03, 0x6b, 0x65, 0x79, 0x12, 0x14, 0x0a, 0x05, 0x76,
	0x61, 0x6c, 0x75, 0x65, 0x18, 0x06, 0x20, 0x01, 0x28, 0x09, 0x52, 0x05, 0x76, 0x61, 0x6c, 0x75,
	0x65, 0x12, 0x18, 0x0a, 0x07, 0x63, 0x72, 0x65, 0x61, 0x74, 0x65, 0x64, 0x18, 0x07, 0x20, 0x01,
	0x28, 0x03, 0x52, 0x07, 0x63, 0x72, 0x65, 0x61, 0x74, 0x65, 0x64, 0x12, 0x1a, 0x0a, 0x08, 0x6d,
	0x6f, 0x64, 0x69, 0x66, 0x69, 0x65, 0x64, 0x18, 0x08, 0x20, 0x01, 0x28, 0x03, 0x52, 0x08, 0x6d,
	0x6f, 0x64, 0x69, 0x66, 0x69, 0x65, 0x64, 0x12, 0x18, 0x0a, 0x07, 0x76, 0x65, 0x72, 0x73, 0x69,
	0x6f, 0x6e, 0x18, 0x09, 0x20, 0x01, 0x28, 0x04, 0x52, 0x07, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6f,
	0x6e, 0x12, 0x12, 0x0a, 0x04, 0x6d, 0x65, 0x74, 0x61, 0x18, 0x0a, 0x20, 0x01, 0x28, 0x0c, 0x52,
	0x04, 0x6d, 0x65, 0x74, 0x61, 0x12, 0x1b, 0x0a, 0x09, 0x75, 0x73, 0x65, 0x72, 0x5f, 0x6d, 0x65,
	0x74, 0x61, 0x18, 0x0b, 0x20, 0x01, 0x28, 0x0c, 0x52, 0x08, 0x75, 0x73, 0x65, 0x72, 0x4d, 0x65,
	0x74, 0x61, 0x22, 0x3a, 0x0a, 0x05, 0x4c, 0x65, 0x76, 0x65, 0x6c, 0x12, 0x08, 0x0a, 0x04, 0x43,
	0x4f, 0x52, 0x45, 0x10, 0x00, 0x12, 0x0a, 0x0a, 0x06, 0x45, 0x4e, 0x47, 0x49, 0x4e, 0x45, 0x10,
	0x01, 0x12, 0x0c, 0x0a, 0x08, 0x53, 0x45, 0x52, 0x56, 0x49, 0x43, 0x45, 0x53, 0x10, 0x02, 0x12,
	0x0d, 0x0a, 0x09, 0x52, 0x45, 0x53, 0x4f, 0x55, 0x52, 0x43, 0x45, 0x53, 0x10, 0x03, 0x22, 0x2c,
	0x0a, 0x08, 0x52, 0x65, 0x73, 0x6f, 0x75, 0x72, 0x63, 0x65, 0x12, 0x0a, 0x0a, 0x06, 0x53, 0x4f,
	0x55, 0x52, 0x43, 0x45, 0x10, 0x00, 0x12, 0x09, 0x0a, 0x05, 0x53, 0x50, 0x41, 0x43, 0x45, 0x10,
	0x01, 0x12, 0x09, 0x0a, 0x05, 0x41, 0x53, 0x53, 0x45, 0x54, 0x10, 0x02, 0x32, 0xd6, 0x02, 0x0a,
	0x03, 0x4c, 0x6f, 0x67, 0x12, 0x3c, 0x0a, 0x07, 0x50, 0x72, 0x6f, 0x64, 0x75, 0x63, 0x65, 0x12,
	0x16, 0x2e, 0x6c, 0x6f, 0x67, 0x2e, 0x76, 0x31, 0x2e, 0x50, 0x72, 0x6f, 0x64, 0x75, 0x63, 0x65,
	0x52, 0x65, 0x71, 0x75, 0x65, 0x73, 0x74, 0x1a, 0x17, 0x2e, 0x6c, 0x6f, 0x67, 0x2e, 0x76, 0x31,
	0x2e, 0x50, 0x72, 0x6f, 0x64, 0x75, 0x63, 0x65, 0x52, 0x65, 0x73, 0x70, 0x6f, 0x6e, 0x73, 0x65,
	0x22, 0x00, 0x12, 0x3c, 0x0a, 0x07, 0x43, 0x6f, 0x6e, 0x73, 0x75, 0x6d, 0x65, 0x12, 0x16, 0x2e,
	0x6c, 0x6f, 0x67, 0x2e, 0x76, 0x31, 0x2e, 0x43, 0x6f, 0x6e, 0x73, 0x75, 0x6d, 0x65, 0x52, 0x65,
	0x71, 0x75, 0x65, 0x73, 0x74, 0x1a, 0x17, 0x2e, 0x6c, 0x6f, 0x67, 0x2e, 0x76, 0x31, 0x2e, 0x43,
	0x6f, 0x6e, 0x73, 0x75, 0x6d, 0x65, 0x52, 0x65, 0x73, 0x70, 0x6f, 0x6e, 0x73, 0x65, 0x22, 0x00,
	0x12, 0x44, 0x0a, 0x0d, 0x43, 0x6f, 0x6e, 0x73, 0x75, 0x6d, 0x65, 0x53, 0x74, 0x72, 0x65, 0x61,
	0x6d, 0x12, 0x16, 0x2e, 0x6c, 0x6f, 0x67, 0x2e, 0x76, 0x31, 0x2e, 0x43, 0x6f, 0x6e, 0x73, 0x75,
	0x6d, 0x65, 0x52, 0x65, 0x71, 0x75, 0x65, 0x73, 0x74, 0x1a, 0x17, 0x2e, 0x6c, 0x6f, 0x67, 0x2e,
	0x76, 0x31, 0x2e, 0x43, 0x6f, 0x6e, 0x73, 0x75, 0x6d, 0x65, 0x52, 0x65, 0x73, 0x70, 0x6f, 0x6e,
	0x73, 0x65, 0x22, 0x00, 0x30, 0x01, 0x12, 0x46, 0x0a, 0x0d, 0x50, 0x72, 0x6f, 0x64, 0x75, 0x63,
	0x65, 0x53, 0x74, 0x72, 0x65, 0x61, 0x6d, 0x12, 0x16, 0x2e, 0x6c, 0x6f, 0x67, 0x2e, 0x76, 0x31,
	0x2e, 0x50, 0x72, 0x6f, 0x64, 0x75, 0x63, 0x65, 0x52, 0x65, 0x71, 0x75, 0x65, 0x73, 0x74, 0x1a,
	0x17, 0x2e, 0x6c, 0x6f, 0x67, 0x2e, 0x76, 0x31, 0x2e, 0x50, 0x72, 0x6f, 0x64, 0x75, 0x63, 0x65,
	0x52, 0x65, 0x73, 0x70, 0x6f, 0x6e, 0x73, 0x65, 0x22, 0x00, 0x28, 0x01, 0x30, 0x01, 0x12, 0x45,
	0x0a, 0x0a, 0x47, 0x65, 0x74, 0x53, 0x65, 0x72, 0x76, 0x65, 0x72, 0x73, 0x12, 0x19, 0x2e, 0x6c,
	0x6f, 0x67, 0x2e, 0x76, 0x31, 0x2e, 0x47, 0x65, 0x74, 0x53, 0x65, 0x72, 0x76, 0x65, 0x72, 0x73,
	0x52, 0x65, 0x71, 0x75, 0x65, 0x73, 0x74, 0x1a, 0x1a, 0x2e, 0x6c, 0x6f, 0x67, 0x2e, 0x76, 0x31,
	0x2e, 0x47, 0x65, 0x74, 0x53, 0x65, 0x72, 0x76, 0x65, 0x72, 0x73, 0x52, 0x65, 0x73, 0x70, 0x6f,
	0x6e, 0x73, 0x65, 0x22, 0x00, 0x42, 0x1c, 0x5a, 0x1a, 0x67, 0x69, 0x74, 0x68, 0x75, 0x62, 0x2e,
	0x63, 0x6f, 0x6d, 0x2f, 0x6e, 0x6f, 0x6d, 0x61, 0x64, 0x2f, 0x70, 0x62, 0x2f, 0x6c, 0x6f, 0x67,
	0x5f, 0x76, 0x31, 0x62, 0x06, 0x70, 0x72, 0x6f, 0x74, 0x6f, 0x33,
}

var (
	file_pb_v1_log_log_proto_rawDescOnce sync.Once
	file_pb_v1_log_log_proto_rawDescData = file_pb_v1_log_log_proto_rawDesc
)

func file_pb_v1_log_log_proto_rawDescGZIP() []byte {
	file_pb_v1_log_log_proto_rawDescOnce.Do(func() {
		file_pb_v1_log_log_proto_rawDescData = protoimpl.X.CompressGZIP(file_pb_v1_log_log_proto_rawDescData)
	})
	return file_pb_v1_log_log_proto_rawDescData
}

var file_pb_v1_log_log_proto_enumTypes = make([]protoimpl.EnumInfo, 2)
var file_pb_v1_log_log_proto_msgTypes = make([]protoimpl.MessageInfo, 11)
var file_pb_v1_log_log_proto_goTypes = []interface{}{
	(Item_Level)(0),            // 0: log.v1.Item.Level
	(Item_Resource)(0),         // 1: log.v1.Item.Resource
	(*GetServersRequest)(nil),  // 2: log.v1.GetServersRequest
	(*GetServersResponse)(nil), // 3: log.v1.GetServersResponse
	(*Server)(nil),             // 4: log.v1.Server
	(*ProduceRequest)(nil),     // 5: log.v1.ProduceRequest
	(*ProduceResponse)(nil),    // 6: log.v1.ProduceResponse
	(*ConsumeRequest)(nil),     // 7: log.v1.ConsumeRequest
	(*ConsumeResponse)(nil),    // 8: log.v1.ConsumeResponse
	(*Record)(nil),             // 9: log.v1.Record
	(*ItemList)(nil),           // 10: log.v1.ItemList
	(*Connections)(nil),        // 11: log.v1.Connections
	(*Item)(nil),               // 12: log.v1.Item
}
var file_pb_v1_log_log_proto_depIdxs = []int32{
	4,  // 0: log.v1.GetServersResponse.servers:type_name -> log.v1.Server
	9,  // 1: log.v1.ProduceRequest.record:type_name -> log.v1.Record
	9,  // 2: log.v1.ConsumeResponse.record:type_name -> log.v1.Record
	12, // 3: log.v1.ItemList.items:type_name -> log.v1.Item
	0,  // 4: log.v1.Item.level:type_name -> log.v1.Item.Level
	1,  // 5: log.v1.Item.type:type_name -> log.v1.Item.Resource
	5,  // 6: log.v1.Log.Produce:input_type -> log.v1.ProduceRequest
	7,  // 7: log.v1.Log.Consume:input_type -> log.v1.ConsumeRequest
	7,  // 8: log.v1.Log.ConsumeStream:input_type -> log.v1.ConsumeRequest
	5,  // 9: log.v1.Log.ProduceStream:input_type -> log.v1.ProduceRequest
	2,  // 10: log.v1.Log.GetServers:input_type -> log.v1.GetServersRequest
	6,  // 11: log.v1.Log.Produce:output_type -> log.v1.ProduceResponse
	8,  // 12: log.v1.Log.Consume:output_type -> log.v1.ConsumeResponse
	8,  // 13: log.v1.Log.ConsumeStream:output_type -> log.v1.ConsumeResponse
	6,  // 14: log.v1.Log.ProduceStream:output_type -> log.v1.ProduceResponse
	3,  // 15: log.v1.Log.GetServers:output_type -> log.v1.GetServersResponse
	11, // [11:16] is the sub-list for method output_type
	6,  // [6:11] is the sub-list for method input_type
	6,  // [6:6] is the sub-list for extension type_name
	6,  // [6:6] is the sub-list for extension extendee
	0,  // [0:6] is the sub-list for field type_name
}

func init() { file_pb_v1_log_log_proto_init() }
func file_pb_v1_log_log_proto_init() {
	if File_pb_v1_log_log_proto != nil {
		return
	}
	if !protoimpl.UnsafeEnabled {
		file_pb_v1_log_log_proto_msgTypes[0].Exporter = func(v interface{}, i int) interface{} {
			switch v := v.(*GetServersRequest); i {
			case 0:
				return &v.state
			case 1:
				return &v.sizeCache
			case 2:
				return &v.unknownFields
			default:
				return nil
			}
		}
		file_pb_v1_log_log_proto_msgTypes[1].Exporter = func(v interface{}, i int) interface{} {
			switch v := v.(*GetServersResponse); i {
			case 0:
				return &v.state
			case 1:
				return &v.sizeCache
			case 2:
				return &v.unknownFields
			default:
				return nil
			}
		}
		file_pb_v1_log_log_proto_msgTypes[2].Exporter = func(v interface{}, i int) interface{} {
			switch v := v.(*Server); i {
			case 0:
				return &v.state
			case 1:
				return &v.sizeCache
			case 2:
				return &v.unknownFields
			default:
				return nil
			}
		}
		file_pb_v1_log_log_proto_msgTypes[3].Exporter = func(v interface{}, i int) interface{} {
			switch v := v.(*ProduceRequest); i {
			case 0:
				return &v.state
			case 1:
				return &v.sizeCache
			case 2:
				return &v.unknownFields
			default:
				return nil
			}
		}
		file_pb_v1_log_log_proto_msgTypes[4].Exporter = func(v interface{}, i int) interface{} {
			switch v := v.(*ProduceResponse); i {
			case 0:
				return &v.state
			case 1:
				return &v.sizeCache
			case 2:
				return &v.unknownFields
			default:
				return nil
			}
		}
		file_pb_v1_log_log_proto_msgTypes[5].Exporter = func(v interface{}, i int) interface{} {
			switch v := v.(*ConsumeRequest); i {
			case 0:
				return &v.state
			case 1:
				return &v.sizeCache
			case 2:
				return &v.unknownFields
			default:
				return nil
			}
		}
		file_pb_v1_log_log_proto_msgTypes[6].Exporter = func(v interface{}, i int) interface{} {
			switch v := v.(*ConsumeResponse); i {
			case 0:
				return &v.state
			case 1:
				return &v.sizeCache
			case 2:
				return &v.unknownFields
			default:
				return nil
			}
		}
		file_pb_v1_log_log_proto_msgTypes[7].Exporter = func(v interface{}, i int) interface{} {
			switch v := v.(*Record); i {
			case 0:
				return &v.state
			case 1:
				return &v.sizeCache
			case 2:
				return &v.unknownFields
			default:
				return nil
			}
		}
		file_pb_v1_log_log_proto_msgTypes[8].Exporter = func(v interface{}, i int) interface{} {
			switch v := v.(*ItemList); i {
			case 0:
				return &v.state
			case 1:
				return &v.sizeCache
			case 2:
				return &v.unknownFields
			default:
				return nil
			}
		}
		file_pb_v1_log_log_proto_msgTypes[9].Exporter = func(v interface{}, i int) interface{} {
			switch v := v.(*Connections); i {
			case 0:
				return &v.state
			case 1:
				return &v.sizeCache
			case 2:
				return &v.unknownFields
			default:
				return nil
			}
		}
		file_pb_v1_log_log_proto_msgTypes[10].Exporter = func(v interface{}, i int) interface{} {
			switch v := v.(*Item); i {
			case 0:
				return &v.state
			case 1:
				return &v.sizeCache
			case 2:
				return &v.unknownFields
			default:
				return nil
			}
		}
	}
	type x struct{}
	out := protoimpl.TypeBuilder{
		File: protoimpl.DescBuilder{
			GoPackagePath: reflect.TypeOf(x{}).PkgPath(),
			RawDescriptor: file_pb_v1_log_log_proto_rawDesc,
			NumEnums:      2,
			NumMessages:   11,
			NumExtensions: 0,
			NumServices:   1,
		},
		GoTypes:           file_pb_v1_log_log_proto_goTypes,
		DependencyIndexes: file_pb_v1_log_log_proto_depIdxs,
		EnumInfos:         file_pb_v1_log_log_proto_enumTypes,
		MessageInfos:      file_pb_v1_log_log_proto_msgTypes,
	}.Build()
	File_pb_v1_log_log_proto = out.File
	file_pb_v1_log_log_proto_rawDesc = nil
	file_pb_v1_log_log_proto_goTypes = nil
	file_pb_v1_log_log_proto_depIdxs = nil
}
